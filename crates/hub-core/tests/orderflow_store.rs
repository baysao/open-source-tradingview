//! The trade store (02-02). AC-2: every Python store scenario captured by
//! `scripts/capture_orderflow_fixtures.py` (segments from REST-format rows, day files from the
//! sliced archive, busy days) gives Python's `trades()` window, coverage, pending flag and
//! after-state through `assemble`. AC-3: the store's lifecycle under tokio's paused clock with an
//! in-memory trade source.

#![allow(clippy::disallowed_methods)] // test arithmetic, not parity code

use futures_util::future::BoxFuture;
use hub_core::clock::WallClock;
use hub_core::orderflow::binance::{parse_agg_trades, Prio};
use hub_core::orderflow::calc::{TradeCols, TradeWindow};
use hub_core::orderflow::trades::{
    assemble, parse_daily_zip, parse_hour, Chunk, StoreState, TradeSource, Trades,
};
use hub_core::orderflow::OfError;
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn fix_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/orderflow")
}

fn read_blob(rel: &str) -> TradeCols {
    let mut z = zip::ZipArchive::new(std::fs::File::open(fix_dir().join(rel)).unwrap()).unwrap();
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

fn day_of(iso: &str) -> i64 {
    parse_hour(&format!("{}00", iso.replace('-', "")))
        .unwrap()
        .div_euclid(86_400_000)
}

fn bits(t: &TradeCols) -> Vec<(i64, u64, u64, i8)> {
    (0..t.len())
        .map(|i| {
            (
                t.t_ms[i],
                t.price[i].to_bits(),
                t.qty[i].to_bits(),
                t.side[i],
            )
        })
        .collect()
}

/// Rebuilds a store exactly as the capture's `_seed_store` did.
fn seed(sc: &Value, manifest: &Value) -> StoreState {
    let mut st = StoreState::default();
    let scen = &sc["scenario"];
    let segs = scen["segments"].as_array().unwrap();
    let pages = sc["inputs"]
        .get("segments")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (seg, pages) in segs.iter().zip(pages) {
        let (a, b) = (seg[0].as_i64().unwrap(), seg[1].as_i64().unwrap());
        let rows: Vec<&Value> = pages
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p.as_array().unwrap())
            .filter(|r| {
                let t = r["T"].as_i64().unwrap();
                a * 1000 <= t && t <= b * 1000 + 999
            })
            .collect();
        if !rows.is_empty() {
            let (ids, cols) = parse_agg_trades(&serde_json::to_vec(&rows).unwrap()).unwrap();
            st.segs.add(Chunk { ids, cols }, a * 1000, b * 1000 + 999);
        }
    }
    let d = day_of(manifest["date"].as_str().unwrap());
    match scen["day"].as_str() {
        Some("ready") => {
            let zip = std::fs::read(
                fix_dir()
                    .join("inputs")
                    .join(sc["inputs"]["daily"].as_str().unwrap()),
            )
            .unwrap();
            st.days.insert(d, Arc::new(parse_daily_zip(&zip).unwrap()));
        }
        Some("busy") => {
            st.day_busy.insert(d);
        }
        _ => {}
    }
    st
}

#[test]
fn store_scenarios_match_python() {
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(fix_dir().join("inputs/manifest.json")).unwrap(),
    )
    .unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(fix_dir().join("stores"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(files.len() >= 5, "store scenarios: {}", files.len());
    let mut checked = 0;
    for f in files {
        let sc: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
        let name = sc["name"].as_str().unwrap().to_string();
        let now_ms = sc["now"].as_f64().unwrap() * 1000.0;
        let mut st = seed(&sc, &manifest);
        for (i, call) in sc["calls"].as_array().unwrap().iter().enumerate() {
            let (t0, t1) = (call["t0"].as_i64().unwrap(), call["t1"].as_i64().unwrap());
            let (win, fx): (TradeWindow, _) = assemble(&st, t0, t1, now_ms);
            let want = read_blob(call["window"].as_str().unwrap());
            assert_eq!(
                bits(&win.trades),
                bits(&want),
                "{name}#{i}: trades or their order"
            );
            let cov: Vec<(i64, i64)> = call["covered"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| (c[0].as_i64().unwrap(), c[1].as_i64().unwrap()))
                .collect();
            assert_eq!(win.covered, cov, "{name}#{i}: covered");
            assert_eq!(
                win.pending,
                call["pending"].as_bool().unwrap(),
                "{name}#{i}: pending"
            );
            if let Some(f) = fx.floor_ms {
                st.floor_ms = Some(f);
            }
            st.day_busy.extend(fx.fetch_days.iter().copied());
            let after = &call["after"];
            assert_eq!(
                st.floor_ms,
                after["floor_ms"].as_f64(),
                "{name}#{i}: floor_ms"
            );
            let busy: Vec<i64> = after["day_busy"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| day_of(d.as_str().unwrap()))
                .collect();
            assert_eq!(
                st.day_busy.iter().copied().collect::<Vec<_>>(),
                busy,
                "{name}#{i}: day_busy"
            );
            let ready: Vec<i64> = after["days_ready"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| day_of(d.as_str().unwrap()))
                .collect();
            assert_eq!(
                st.days.keys().copied().collect::<Vec<_>>(),
                ready,
                "{name}#{i}: days_ready"
            );
            checked += 1;
            println!(
                "  pass  {name}#{i}: {} trades, pending {}",
                win.trades.len(),
                win.pending
            );
        }
    }
    println!("store scenarios: {checked} calls equal Python");
}

// ---------------------------------------------------------------- lifecycle (AC-3)

const T_START: f64 = 1_795_003_200_000.0 + 1_800_000.0; // 12:30 UTC
const T0_TRADES: i64 = 1_795_003_200_000 - 7_200_000; // trades from 10:00
const STEP_MS: i64 = 100; // one trade every 100 ms

/// Trade `i` happens at `T0_TRADES + i * STEP_MS`; it exists once the wall clock has passed it.
struct Synthetic {
    clock: WallClock,
    pages: AtomicUsize,
    newest_calls: AtomicUsize,
    day_calls: AtomicUsize,
    day_fails: bool,
}

impl Synthetic {
    fn max_id(&self) -> i64 {
        ((self.clock.now_ms() as i64 - T0_TRADES) / STEP_MS).max(0)
    }

    fn page(&self, from: i64, to: i64) -> Option<Chunk> {
        if from > to {
            return None;
        }
        let ids: Vec<i64> = (from..=to).collect();
        let cols = TradeCols {
            t_ms: ids.iter().map(|i| T0_TRADES + i * STEP_MS).collect(),
            price: ids.iter().map(|i| 100.0 + (*i % 50) as f64 * 0.5).collect(),
            qty: vec![0.25; ids.len()],
            side: ids
                .iter()
                .map(|i| if i % 3 == 0 { -1 } else { 1 })
                .collect(),
        };
        Some(Chunk { ids, cols })
    }
}

impl TradeSource for Synthetic {
    fn agg_trades<'a>(
        &'a self,
        _sym: &'a str,
        from_id: Option<i64>,
        _p: Prio,
    ) -> BoxFuture<'a, Result<Option<Chunk>, OfError>> {
        Box::pin(async move {
            self.pages.fetch_add(1, Ordering::SeqCst);
            let max = self.max_id();
            Ok(match from_id {
                None => {
                    self.newest_calls.fetch_add(1, Ordering::SeqCst);
                    self.page((max - 999).max(0), max)
                }
                Some(f) => self.page(f, (f + 999).min(max)),
            })
        })
    }

    fn daily_zip<'a>(
        &'a self,
        _sym: &'a str,
        _day: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, OfError>> {
        Box::pin(async move {
            self.day_calls.fetch_add(1, Ordering::SeqCst);
            if self.day_fails {
                Err(OfError::value("archive not published yet"))
            } else {
                Ok(Vec::new())
            }
        })
    }
}

fn temp_cache(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("of-store-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn ids_once_in_order(w: &TradeWindow) -> bool {
    w.trades.t_ms.windows(2).all(|p| p[1] == p[0] + STEP_MS)
}

#[tokio::test(start_paused = true)]
async fn store_lifecycle() {
    let cache = temp_cache("life");
    let clock = WallClock::frozen(T_START);
    let src = Arc::new(Synthetic {
        clock,
        pages: AtomicUsize::new(0),
        newest_calls: AtomicUsize::new(0),
        day_calls: AtomicUsize::new(0),
        day_fails: true,
    });
    let trades = Trades::new(src.clone(), clock, Some(cache.clone()));
    let (t0, t1) = (T0_TRADES + 30 * 60_000, T_START as i64);

    let first = trades.window("BTCUSDT", t0, t1).await;
    assert!(
        first.pending && first.trades.is_empty(),
        "nothing loaded yet"
    );
    assert!(
        trades.running("BTCUSDT"),
        "the loops start on the first request"
    );

    tokio::time::sleep(Duration::from_secs(120)).await;
    let w = trades.window("BTCUSDT", t0, t1).await;
    assert!(
        !w.pending,
        "back-fill and head poll cover the window within two minutes"
    );
    assert!(ids_once_in_order(&w), "every trade once, in order");
    assert_eq!(w.trades.t_ms.first().copied(), Some(t0), "starts at t0");

    let heads_before = src.newest_calls.load(Ordering::SeqCst);
    let pages_before = src.pages.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(20)).await;
    trades.window("BTCUSDT", t0, t1).await;
    assert!(
        src.pages.load(Ordering::SeqCst) - pages_before >= 5,
        "the head polls about every 4 s"
    );
    assert_eq!(
        src.newest_calls.load(Ordering::SeqCst),
        heads_before,
        "a fresh head polls by fromId, never the newest page"
    );

    let hours: Vec<_> = std::fs::read_dir(cache.join("hours/BTCUSDT"))
        .map(|d| d.filter_map(Result::ok).collect())
        .unwrap_or_default();
    assert!(!hours.is_empty(), "complete hours saved after 60 s");
    assert!(
        hours
            .iter()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".part")),
        "no half-written file left"
    );

    tokio::time::sleep(Duration::from_secs(601)).await;
    assert!(
        !trades.running("BTCUSDT"),
        "the loops stop after 600 s without a request"
    );
    trades.window("BTCUSDT", t0, t1).await;
    assert!(
        trades.running("BTCUSDT"),
        "a request after the idle stop starts them again"
    );

    let reopened = Trades::new(src.clone(), clock, Some(cache.clone()));
    let _ = reopened.window("BTCUSDT", t0, t0 + 1).await;
    let st = reopened.state("BTCUSDT").unwrap();
    assert!(
        !st.segs.segs.is_empty(),
        "a new store reloads the saved hours before any REST call"
    );

    // the archive of the day before yesterday fails: retried only after 30 minutes (a day older
    // than the 30 h REST reach, so the REST part does not make the window pending)
    let y0 = T0_TRADES - 2 * 86_400_000;
    let calls_before = src.day_calls.load(Ordering::SeqCst);
    trades.window("BTCUSDT", y0, y0 + 60_000).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(src.day_calls.load(Ordering::SeqCst), calls_before + 1);
    let w = trades.window("BTCUSDT", y0, y0 + 60_000).await;
    assert!(
        !w.pending,
        "a recently failed day is not retried and not pending"
    );
    assert_eq!(src.day_calls.load(Ordering::SeqCst), calls_before + 1);
    tokio::time::sleep(Duration::from_secs(1800)).await;
    trades.window("BTCUSDT", y0, y0 + 60_000).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(
        src.day_calls.load(Ordering::SeqCst),
        calls_before + 2,
        "retried after 30 minutes"
    );

    // dropping every handle stops the loops
    drop(trades);
    drop(reopened);
    tokio::time::sleep(Duration::from_secs(10)).await;
    let after_drop = src.pages.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        src.pages.load(Ordering::SeqCst),
        after_drop,
        "no request after the store is dropped"
    );
    let _ = std::fs::remove_dir_all(&cache);
    let _ = Mutex::new(()); // keep the import list honest across edits
}
