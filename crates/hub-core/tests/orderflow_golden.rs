//! Order-flow golden fixtures (02-01): every case captured from `hub/orderflow_api.py` by
//! `scripts/capture_orderflow_fixtures.py` is replayed through the pure `calc` functions, with the
//! route's argument parsing and the engine's `_rows` cache rules, and no network.
//!
//! Per call it checks (AC-2) that every `_rows` call has Python's arguments and Python's fetch
//! plan, and (AC-3) that the reply equals Python's JSON: values bit for bit, int vs float at every
//! path, key order. The 1e-9 relative allowance applies only to the unrounded paths and is printed
//! as `tolerance used at <path>` whenever it was needed.

#![allow(clippy::disallowed_methods)] // the comparator's own arithmetic, not parity code

use hub_core::orderflow::binance::parse_klines;
use hub_core::orderflow::calc::{
    auto_row_size, delta, delta_window, estimated_rows, footprint, footprint_spans, intrabar,
    need_volume, profile, profile_window, py_float, py_int, real_rows, resolve, rows_merge,
    rows_need, rows_window, shift, FootprintArgs, FootprintData, LowerRows, ProfileOpts, Span,
    TradeCols, TradeWindow,
};
use hub_core::orderflow::{OfError, Pick, TfArg};
use hub_types::market::{Bar, Tf};
use hub_types::orderflow::{FlowSource, Num};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

const UNROUNDED: [&str; 6] = ["lo", "step", "poc", "vah", "val", "row_size"];

fn fix_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orderflow")
}

fn load(rel: &str) -> Value {
    let p = fix_dir().join(rel);
    serde_json::from_str(
        &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display())),
    )
    .unwrap()
}

fn futures() -> BTreeSet<String> {
    load("inputs/manifest.json")["futures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect()
}

fn read_blob(rel: &str) -> TradeCols {
    let f = std::fs::File::open(fix_dir().join(rel)).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut text = String::new();
    z.by_index(0).unwrap().read_to_string(&mut text).unwrap();
    let mut t = TradeCols::default();
    for line in text.lines().skip(1) {
        let c: Vec<&str> = line.split(',').collect();
        t.t_ms.push(c[0].parse().unwrap());
        t.price.push(c[1].parse().unwrap());
        t.qty.push(c[2].parse().unwrap());
        t.side.push(c[3].parse().unwrap());
    }
    t
}

/// One process' lower-bar cache (Python's `_cache`), keyed `(source, raw ticker, tf)`.
#[derive(Default)]
struct Engine {
    cache: HashMap<(FlowSource, String, Tf), LowerRows>,
}

/// One call being replayed: its recorded `_rows` calls are consumed in order.
struct Call<'a> {
    rec: &'a Value,
    rows_i: usize,
}

impl Engine {
    fn rows(
        &mut self,
        call: &mut Call<'_>,
        src: FlowSource,
        ticker: &str,
        tf: Tf,
        span: Span,
        ctx: &str,
    ) -> LowerRows {
        let rec = &call.rec["rows_calls"][call.rows_i];
        call.rows_i += 1;
        let src_s = match src {
            FlowSource::Real => "real",
            FlowSource::Estimated => "estimated",
        };
        assert_eq!(
            (
                rec["src"].as_str(),
                rec["ticker"].as_str(),
                rec["tf"].as_str(),
                rec["a"].as_i64(),
                rec["b"].as_i64()
            ),
            (
                Some(src_s),
                Some(ticker),
                Some(tf.as_str()),
                Some(span.a),
                Some(span.b)
            ),
            "{ctx}: _rows call #{} differs from Python's",
            call.rows_i - 1
        );
        let have = self.cache.entry((src, ticker.to_string(), tf)).or_default();
        let need = rows_need(have, tf.secs(), span.a, span.b);
        let fetches = rec["fetches"].as_array().unwrap();
        let plan: Vec<(i64, i64)> = need.iter().map(|s| (s.a, s.b)).collect();
        let want: Vec<(i64, i64)> = fetches
            .iter()
            .map(|f| (f["lo"].as_i64().unwrap(), f["hi"].as_i64().unwrap()))
            .collect();
        assert_eq!(
            plan,
            want,
            "{ctx}: fetch plan of _rows #{}",
            call.rows_i - 1
        );
        for (s, f) in need.iter().zip(fetches) {
            let got = match f["kind"].as_str().unwrap() {
                "real" => {
                    let pages: Vec<_> = f["pages"]
                        .as_array()
                        .map(|ps| {
                            ps.iter()
                                .map(|p| parse_klines(&serde_json::to_vec(p).unwrap()).unwrap())
                                .collect()
                        })
                        .unwrap_or_default();
                    real_rows(&pages)
                }
                _ => {
                    let chunks: Vec<Vec<Bar>> =
                        serde_json::from_value(f.get("chunks").cloned().unwrap_or(json!([])))
                            .unwrap();
                    estimated_rows(&chunks, *s)
                }
            };
            rows_merge(have, got);
        }
        rows_window(have, span)
    }
}

fn q<'a>(query: &'a Value, k: &str) -> Option<&'a str> {
    query.get(k).and_then(Value::as_str)
}

fn int_arg(query: &Value, k: &str, default: i64) -> Result<i64, OfError> {
    q(query, k).map_or(Ok(default), py_int)
}

fn float_arg(query: &Value, k: &str, default: f64) -> Result<f64, OfError> {
    q(query, k).map_or(Ok(default), py_float)
}

fn run(
    engine: &mut Engine,
    fut: &BTreeSet<String>,
    rec: &Value,
    now: f64,
    ctx: &str,
) -> (u16, Value) {
    let mut call = Call { rec, rows_i: 0 };
    let res = match rec["kind"].as_str().unwrap() {
        "delta" => run_delta(engine, &mut call, fut, now, ctx),
        "profile" => run_profile(engine, &mut call, fut, now, ctx),
        _ => run_footprint(engine, &mut call, fut, now, ctx),
    };
    assert_eq!(
        call.rows_i,
        rec["rows_calls"].as_array().unwrap().len(),
        "{ctx}: Python made more _rows calls"
    );
    match res {
        Ok(v) => (200, v),
        Err(e) => (502, json!({"error": e.to_string()})),
    }
}

fn run_delta(
    engine: &mut Engine,
    call: &mut Call<'_>,
    fut: &BTreeSet<String>,
    now: f64,
    ctx: &str,
) -> Result<Value, OfError> {
    let query = &call.rec["query"];
    let now = now as i64;
    let (from, to) = (
        int_arg(query, "from", now - 86_400)?,
        int_arg(query, "to", now)?,
    );
    let ticker = q(query, "symbol").unwrap_or("frxXAUUSD");
    let r = resolve(ticker, Pick::from_param(q(query, "source")), fut)?;
    let tf = TfArg(q(query, "tf").unwrap_or("M5").to_string()).resolve()?;
    let lower = engine.rows(
        call,
        r.src,
        ticker,
        intrabar(tf),
        delta_window(tf, from, to),
        ctx,
    );
    if r.src == FlowSource::Estimated {
        need_volume(&lower)?;
    }
    Ok(serde_json::to_value(delta(tf, &lower, &r)).unwrap())
}

fn run_profile(
    engine: &mut Engine,
    call: &mut Call<'_>,
    fut: &BTreeSet<String>,
    now: f64,
    ctx: &str,
) -> Result<Value, OfError> {
    let query = &call.rec["query"];
    let now = now as i64;
    let (from, to) = (
        int_arg(query, "from", now - 86_400)?,
        int_arg(query, "to", now)?,
    );
    let opts = ProfileOpts {
        mode: q(query, "mode").unwrap_or("sessions").to_string(),
        rows: int_arg(query, "rows", 24)?,
        row_size: float_arg(query, "row_size", 0.0)?,
        va_pct: float_arg(query, "va", 70.0)? / 100.0,
        developing: q(query, "developing").unwrap_or("1") == "1",
    };
    let ticker = q(query, "symbol").unwrap_or("frxXAUUSD");
    let r = resolve(ticker, Pick::from_param(q(query, "source")), fut)?;
    let tf = TfArg(q(query, "tf").unwrap_or("M5").to_string()).resolve()?;
    let w = profile_window(tf, &opts.mode, r.src, from, to);
    let lower = engine.rows(call, r.src, ticker, w.itf, w.lower, ctx);
    let chart = match (r.src, w.chart) {
        (FlowSource::Real, Some(span)) => {
            Some(engine.rows(call, FlowSource::Estimated, ticker, tf, span, ctx))
        }
        _ => {
            need_volume(&lower)?;
            None
        }
    };
    Ok(serde_json::to_value(profile(&w, &opts, &r, &lower, chart.as_ref())?).unwrap())
}

fn run_footprint(
    engine: &mut Engine,
    call: &mut Call<'_>,
    fut: &BTreeSet<String>,
    now: f64,
    ctx: &str,
) -> Result<Value, OfError> {
    let rec = call.rec;
    let query = &rec["query"];
    let nowi = now as i64;
    let ticker = q(query, "symbol").unwrap_or("frxXAUUSD");
    let (a, b) = (
        int_arg(query, "from", nowi - 3 * 3600)?,
        int_arg(query, "to", nowi)?,
    );
    let tf = TfArg(q(query, "tf").unwrap_or("M5").to_string()).resolve()?;
    let sec = tf.secs();
    let n = 1000.min(5.max((b - a).div_euclid(sec) + 5));
    let rh = &rec["route_history"];
    assert_eq!(
        (
            rh["ticker"].as_str(),
            rh["tf"].as_str(),
            rh["n"].as_i64(),
            rh["to_ms"].as_i64()
        ),
        (Some(ticker), Some(tf.as_str()), Some(n), Some(b * 1000)),
        "{ctx}: the route's chart-bars call differs"
    );
    let bars: Vec<Bar> = serde_json::from_value(rh["bars"].clone()).unwrap();
    let cb: Vec<(i64, f64, f64)> = bars
        .iter()
        .filter(|x| a <= x.time && x.time <= b)
        .map(|x| (x.time, x.high, x.low))
        .collect();
    let row_size_f = float_arg(query, "row_size", 0.0)?;
    let fill = q(query, "fill").unwrap_or("1") != "0";
    let r = resolve(ticker, Pick::from_param(q(query, "source")), fut)?;
    let row_size = if row_size_f <= 0.0 {
        auto_row_size(&cb)
    } else {
        Num::Float(row_size_f)
    };
    let args = FootprintArgs {
        tf,
        from_s: a,
        to_s: b,
        row_size,
        fill,
    };
    let (m1_span, chart_span, (t0, t1)) = footprint_spans(tf, a, b);
    let reply = match r.src {
        FlowSource::Real => {
            let m1 = engine.rows(call, FlowSource::Real, ticker, Tf::M1, m1_span, ctx);
            let chart = engine.rows(call, FlowSource::Estimated, ticker, tf, chart_span, ctx);
            let g_at = shift(&m1, &chart, sec, 60);
            let tr = &rec["trades"];
            assert_eq!(
                (tr["sym"].as_str(), tr["t0"].as_i64(), tr["t1"].as_i64()),
                (r.real.as_deref(), Some(t0), Some(t1)),
                "{ctx}: the trades() call differs"
            );
            let window = TradeWindow {
                trades: read_blob(tr["window"].as_str().unwrap()),
                covered: tr["covered"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| (c[0].as_i64().unwrap(), c[1].as_i64().unwrap()))
                    .collect(),
                pending: tr["pending"].as_bool().unwrap(),
            };
            footprint(
                &args,
                FootprintData::Real {
                    m1: &m1,
                    g_at: &g_at,
                    window: &window,
                },
                now,
                &r,
            )
        }
        FlowSource::Estimated => {
            let m1 = engine.rows(call, FlowSource::Estimated, ticker, Tf::M1, m1_span, ctx);
            need_volume(&m1)?;
            footprint(&args, FootprintData::Estimated { m1: &m1 }, now, &r)
        }
    };
    Ok(serde_json::to_value(reply?).unwrap())
}

/// Bit-exact comparison with number kinds and key order; the allowance is only for UNROUNDED keys.
fn assert_same(
    want: &Value,
    got: &Value,
    path: &str,
    used: &mut Vec<String>,
    bad: &mut Vec<String>,
) {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            let (wk, gk): (Vec<_>, Vec<_>) = (w.keys().collect(), g.keys().collect());
            if wk != gk {
                bad.push(format!("{path}: keys {wk:?} vs {gk:?}"));
                return;
            }
            for (k, v) in w {
                assert_same(v, &g[k], &format!("{path}.{k}"), used, bad);
            }
        }
        (Value::Array(w), Value::Array(g)) => {
            if w.len() != g.len() {
                bad.push(format!("{path}: length {} vs {}", w.len(), g.len()));
                return;
            }
            for (i, (a, b)) in w.iter().zip(g).enumerate() {
                assert_same(a, b, &format!("{path}[{i}]"), used, bad);
            }
        }
        (Value::Number(w), Value::Number(g)) => {
            if w.is_f64() != g.is_f64() {
                bad.push(format!("{path}: number kind {w} vs {g}"));
            } else if w.is_f64() {
                let (a, b) = (w.as_f64().unwrap(), g.as_f64().unwrap());
                if a.to_bits() != b.to_bits() {
                    let key = path
                        .rsplit('.')
                        .next()
                        .unwrap_or("")
                        .split('[')
                        .next()
                        .unwrap_or("");
                    let rel = (a - b).abs() / a.abs().max(b.abs()).max(f64::MIN_POSITIVE);
                    if UNROUNDED.contains(&key) && rel <= 1e-9 {
                        used.push(format!("tolerance used at {path}: {a:?} vs {b:?}"));
                    } else {
                        bad.push(format!("{path}: {a:?} vs {b:?}"));
                    }
                }
            } else if w != g {
                bad.push(format!("{path}: {w} vs {g}"));
            }
        }
        (w, g) if w == g => {}
        (w, g) => bad.push(format!("{path}: {w} vs {g}")),
    }
}

#[test]
fn every_case_matches_python() {
    let fut = futures();
    let mut dir: Vec<PathBuf> = std::fs::read_dir(fix_dir().join("cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    dir.sort();
    assert!(
        dir.len() >= 18,
        "expected the captured cases, found {}",
        dir.len()
    );
    let (mut used, mut failures, mut calls) = (Vec::new(), Vec::new(), 0);
    for path in &dir {
        let case: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let name = case["name"].as_str().unwrap();
        let now = case["now"].as_f64().unwrap();
        let mut engine = Engine::default(); // a cold cache per case, as the capture
        for (i, rec) in case["calls"].as_array().unwrap().iter().enumerate() {
            let ctx = format!("{name}#{i}");
            let (status, body) = run(&mut engine, &fut, rec, now, &ctx);
            if std::env::var("OF_DUMP").ok().as_deref() == Some(ctx.as_str()) {
                std::fs::write(
                    std::env::temp_dir().join("of_dump.json"),
                    serde_json::to_string(&body).unwrap(),
                )
                .unwrap();
            }
            let want = &rec["expected"];
            let mut bad = Vec::new();
            if Some(u64::from(status)) != want["status"].as_u64() {
                bad.push(format!("status {status} vs {}: {body}", want["status"]));
            } else {
                assert_same(&want["body"], &body, "body", &mut used, &mut bad);
            }
            calls += 1;
            if !bad.is_empty() {
                failures.push(format!(
                    "{ctx}: {} differences, first: {:?}",
                    bad.len(),
                    &bad[..bad.len().min(3)]
                ));
            } else {
                println!("  pass  {ctx} ({status})");
            }
        }
    }
    for u in &used {
        println!("{u}");
    }
    println!(
        "golden: {} calls, {} failed, {} tolerance used",
        calls,
        failures.len(),
        used.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
