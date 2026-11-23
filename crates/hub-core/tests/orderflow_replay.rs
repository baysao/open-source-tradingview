//! The order-flow service against fakes (02-02, AC-4): every 02-01 fixture case runs through
//! `Ask::parse` and `OrderFlow::answer`, with Python's recorded klines served by a local fake fapi,
//! Python's recorded chart history behind `BarsCache` (an unrecorded call fails the test), the
//! recorded trade windows, and the frozen clock. The reply must equal Python's JSON.

mod common;

use common::{assert_same, case_files, load, read_blob};
use futures_util::future::BoxFuture;
use hub_core::bars_cache::BarsCache;
use hub_core::clock::WallClock;
use hub_core::market::{BarsQuery, FeedError};
use hub_core::orderflow::binance::{Fapi, FuturesList};
use hub_core::orderflow::calc::TradeWindow;
use hub_core::orderflow::{Ask, Kind, OrderFlow, TradeWindows};
use hub_core::sources::HistoryFetch;
use hub_types::market::{Bar, Source, Tf};
use serde_json::{json, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

type HistKey = (Source, Tf, i64, i64);

/// Python's chart history calls, keyed as the hub's `BarsQuery` sees them (limit capped at 1,000
/// for Binance and Bybit). Every lookup that misses is recorded and fails the test.
struct ReplayHistory {
    pages: HashMap<HistKey, Vec<Bar>>,
    missed: Mutex<Vec<String>>,
}

fn source_of(feed: &str) -> Source {
    match feed {
        "deriv" => Source::Deriv,
        "bybit" => Source::Bybit,
        _ => Source::Binance,
    }
}

impl HistoryFetch for ReplayHistory {
    fn fetch<'a>(&'a self, q: &'a BarsQuery) -> BoxFuture<'a, Result<Vec<Bar>, FeedError>> {
        Box::pin(async move {
            let key = (q.source, q.tf, q.limit, q.to_ms.unwrap_or(-1));
            match self.pages.get(&key) {
                Some(b) => Ok(b.clone()),
                None => {
                    self.missed.lock().unwrap().push(format!("{key:?}"));
                    Err(FeedError::new(format!("unrecorded history call {key:?}")))
                }
            }
        })
    }
}

struct Recorded(HashMap<(String, i64, i64), TradeWindow>);

impl TradeWindows for Recorded {
    fn window<'a>(&'a self, sym: &'a str, t0_ms: i64, t1_ms: i64) -> BoxFuture<'a, TradeWindow> {
        Box::pin(async move {
            self.0
                .get(&(sym.to_string(), t0_ms, t1_ms))
                .cloned()
                .unwrap_or_else(|| panic!("unrecorded trades({sym}, {t0_ms}, {t1_ms})"))
        })
    }
}

/// A fake fapi serving the recorded klines pages by their exact query.
async fn fake_fapi(pages: HashMap<String, Value>) -> String {
    use axum::extract::{Query, State};
    use axum::routing::get;
    let pages = Arc::new(pages);
    let app = axum::Router::new()
        .route(
            "/fapi/v1/klines",
            get(
                |State(p): State<Arc<HashMap<String, Value>>>,
                 Query(q): Query<HashMap<String, String>>| async move {
                    let key = format!(
                        "symbol={}&interval={}&startTime={}&endTime={}",
                        q["symbol"], q["interval"], q["startTime"], q["endTime"]
                    );
                    match p.get(&key) {
                        Some(v) => axum::Json(v.clone()).into_response(),
                        None => (
                            axum::http::StatusCode::IM_A_TEAPOT,
                            format!("unrecorded klines {key}"),
                        )
                            .into_response(),
                    }
                },
            ),
        )
        .with_state(pages);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

use axum::response::IntoResponse;

fn klines_key(url: &str) -> String {
    let q = url.split_once('?').map_or("", |(_, q)| q);
    let p: HashMap<&str, &str> = q.split('&').filter_map(|kv| kv.split_once('=')).collect();
    format!(
        "symbol={}&interval={}&startTime={}&endTime={}",
        p["symbol"], p["interval"], p["startTime"], p["endTime"]
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_case_through_the_service_matches_python() {
    let futures: BTreeSet<String> = load("inputs/manifest.json")["futures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap().to_string())
        .collect();
    let (mut used, mut failures, mut calls) = (Vec::new(), Vec::new(), 0);
    for path in case_files() {
        let case: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let name = case["name"].as_str().unwrap().to_string();
        let tape = load(&format!("inputs/tapes/{name}.json"));
        let (mut klines, mut hist) = (HashMap::new(), HashMap::new());
        for e in tape.as_array().unwrap() {
            let req = &e["req"];
            match e["seam"].as_str().unwrap() {
                "klines" => {
                    klines.insert(klines_key(req["url"].as_str().unwrap()), e["resp"].clone());
                }
                "history" => {
                    let src = source_of(req["feed"].as_str().unwrap());
                    let n = req["n"].as_i64().unwrap();
                    let limit = if src == Source::Deriv { n } else { n.min(1000) };
                    let tf = Tf::parse(req["tf"].as_str().unwrap()).unwrap();
                    let bars: Vec<Bar> = serde_json::from_value(e["resp"].clone()).unwrap();
                    hist.insert((src, tf, limit, req["to_ms"].as_i64().unwrap()), bars);
                }
                _ => {}
            }
        }
        let mut windows = HashMap::new();
        for c in case["calls"].as_array().unwrap() {
            if let Some(tr) = c.get("trades").filter(|t| !t.is_null()) {
                let w = TradeWindow {
                    trades: read_blob(tr["window"].as_str().unwrap()),
                    covered: tr["covered"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| (p[0].as_i64().unwrap(), p[1].as_i64().unwrap()))
                        .collect(),
                    pending: tr["pending"].as_bool().unwrap(),
                };
                windows.insert(
                    (
                        tr["sym"].as_str().unwrap().to_string(),
                        tr["t0"].as_i64().unwrap(),
                        tr["t1"].as_i64().unwrap(),
                    ),
                    w,
                );
            }
        }
        let base = fake_fapi(klines).await;
        let replay = Arc::new(ReplayHistory {
            pages: hist,
            missed: Mutex::default(),
        });
        let fut = futures.clone();
        let list = FuturesList::new(Arc::new(move || {
            let f = fut.clone();
            Box::pin(async move { Ok(f) })
        }));
        let clock = WallClock::frozen(case["now"].as_f64().unwrap() * 1000.0);
        let of = OrderFlow::with_parts(
            BarsCache::new(replay.clone()),
            Fapi::new(&base, "http://127.0.0.1:9"),
            list,
            Arc::new(Recorded(windows)),
            clock,
        );
        for (i, c) in case["calls"].as_array().unwrap().iter().enumerate() {
            let ctx = format!("{name}#{i}");
            let kind = match c["kind"].as_str().unwrap() {
                "delta" => Kind::Delta,
                "profile" => Kind::Profile,
                _ => Kind::Footprint,
            };
            let q: HashMap<String, String> = serde_json::from_value(c["query"].clone()).unwrap();
            let res = match Ask::parse(kind, &q, of.now_s()) {
                Ok(ask) => of.answer(&ask).await,
                Err(e) => Err(e),
            };
            let (status, body) = match res {
                Ok(r) => (200, serde_json::to_value(r).unwrap()),
                Err(e) => (502, json!({"error": e.to_string()})),
            };
            let want = &c["expected"];
            let mut bad = Vec::new();
            if Some(status) != want["status"].as_u64() {
                bad.push(format!("status {status} vs {}: {body}", want["status"]));
            } else {
                assert_same(&want["body"], &body, "body", &mut used, &mut bad);
            }
            let missed = replay.missed.lock().unwrap().clone();
            if !missed.is_empty() {
                bad.push(format!("unrecorded history calls: {missed:?}"));
            }
            calls += 1;
            if bad.is_empty() {
                println!("  pass  {ctx} ({status})");
            } else {
                failures.push(format!(
                    "{ctx}: {} differences, first: {:?}",
                    bad.len(),
                    &bad[..bad.len().min(3)]
                ));
            }
        }
    }
    println!(
        "replay: {calls} calls, {} failed, {} tolerance used",
        failures.len(),
        used.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
