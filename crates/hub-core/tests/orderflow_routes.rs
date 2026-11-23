//! The three `/api/orderflow/*` routes in-process (02-03, AC-1): Python's keys on 200, Python's
//! error texts on 502, `feed=mt5` (D-009), the D-009 guards, one warn line in `/api/logs` per
//! error, and `Cache-Control: no-store`. A local fake serves exchangeInfo, empty klines and
//! aggTrades pages, and Binance spot candles; nothing reaches the network.

use axum::routing::get;
use hub_core::config::Config;
use hub_core::logbus::RingLayer;
use hub_core::sources::Endpoints;
use hub_core::AppState;
use serde_json::{json, Value};
use tracing_subscriber::prelude::*;

async fn fake_upstream() -> String {
    let spot = || async {
        let t0: i64 = 1_795_000_000_000;
        let rows: Vec<Value> = (0..30)
            .map(|i| {
                json!([
                    t0 + i * 300_000,
                    "100.0",
                    "101.5",
                    "99.5",
                    "100.5",
                    "12.0",
                    0,
                    "0",
                    1,
                    "6.0",
                    "0",
                    "0"
                ])
            })
            .collect();
        axum::Json(Value::Array(rows))
    };
    let app = axum::Router::new()
        .route(
            "/fapi/v1/exchangeInfo",
            get(|| async {
                axum::Json(json!({"symbols": [
                    {"symbol": "BTCUSDT", "status": "TRADING"},
                    {"symbol": "XAUUSDT", "status": "TRADING"}
                ]}))
            }),
        )
        .route("/fapi/v1/klines", get(|| async { axum::Json(json!([])) }))
        .route(
            "/fapi/v1/aggTrades",
            get(|| async { axum::Json(json!([])) }),
        )
        .route("/api/v3/klines", get(spot));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn hub(fake: &str) -> (String, AppState) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = Config::from_lookup(|_| None).unwrap();
    cfg.http_port = port;
    let ep = Endpoints {
        binance_rest: fake.to_string(),
        bybit_rest: "http://127.0.0.1:9".into(),
        deriv_ws: "ws://127.0.0.1:9".into(),
        binance_fapi: fake.to_string(),
        binance_vision: "http://127.0.0.1:9".into(),
        ..Endpoints::default()
    };
    let state = AppState::build_with(cfg, ep);
    let s = state.clone();
    tokio::spawn(async move { hub_core::serve(listener, s).await.unwrap() });
    (format!("http://127.0.0.1:{port}"), state)
}

async fn get_json(c: &reqwest::Client, url: &str) -> (u16, Option<String>, Value) {
    let r = c.get(url).send().await.unwrap();
    let status = r.status().as_u16();
    let cache = r
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    (status, cache, r.json().await.unwrap())
}

fn keys(v: &Value) -> Vec<String> {
    v.as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

#[tokio::test]
async fn routes_answer_like_python() {
    let fake = fake_upstream().await;
    let (base, state) = hub(&fake).await;
    let _log = tracing_subscriber::registry()
        .with(RingLayer::new(state.logs.clone()))
        .set_default();
    let c = reqwest::Client::new();
    let past = "from=1795000000&to=1795008000";

    let (s, cache, d) = get_json(
        &c,
        &format!("{base}/api/orderflow/delta?symbol=BTCUSD&feed=binance&tf=M5&{past}"),
    )
    .await;
    assert_eq!((s, cache.as_deref()), (200, Some("no-store")));
    assert_eq!(
        keys(&d),
        [
            "rows",
            "source",
            "real_symbol",
            "intrabar",
            "unit",
            "method"
        ]
    );
    assert_eq!(
        (d["source"].as_str(), d["real_symbol"].as_str()),
        (Some("real"), Some("BTCUSDT"))
    );

    let (s, _, p) = get_json(
        &c,
        &format!("{base}/api/orderflow/profile?symbol=BTCUSD&feed=binance&tf=M5&{past}"),
    )
    .await;
    assert_eq!(s, 200, "{p}");
    assert_eq!(
        keys(&p),
        [
            "sessions",
            "source",
            "real_symbol",
            "intrabar",
            "unit",
            "mode",
            "va_pct",
            "method",
            "shift"
        ]
    );

    let (s, _, f) = get_json(
        &c,
        &format!("{base}/api/orderflow/footprint?symbol=BTCUSD&feed=binance&tf=M5&{past}"),
    )
    .await;
    assert_eq!(s, 200, "{f}");
    assert_eq!(
        keys(&f),
        [
            "candles",
            "source",
            "real_symbol",
            "row_size",
            "unit",
            "covered_from",
            "pending",
            "est_candles",
            "method"
        ]
    );

    let errors = [
        ("delta", "symbol=BTCUSD&tf=M2", "KeyError: 'M2'"),
        (
            "delta",
            "symbol=BTCUSD&from=abc",
            "ValueError: invalid literal for int() with base 10: 'abc'",
        ),
        (
            "delta",
            "symbol=BYBIT_MNTUSDT&source=real",
            "ValueError: no real buyer/seller data for BYBIT_MNTUSDT",
        ),
        (
            "profile",
            "symbol=BTCUSD&feed=mt5",
            "RuntimeError: mt5 history is not available in this server (D-001)",
        ),
        (
            "profile",
            "symbol=BTCUSD&rows=200000",
            "ValueError: rows must be 100000 or fewer",
        ),
        (
            "profile",
            "symbol=BTCUSD&va=nan",
            "ValueError: row_size and va must be finite numbers",
        ),
        (
            "footprint",
            "symbol=BTCUSD&feed=arm",
            "RuntimeError: arm history is not available in this server (D-001)",
        ),
    ];
    for (kind, q, want) in errors {
        let (s, cache, b) = get_json(&c, &format!("{base}/api/orderflow/{kind}?{q}")).await;
        assert_eq!(
            (s, b["error"].as_str(), cache.as_deref()),
            (502, Some(want), Some("no-store")),
            "{kind}?{q}"
        );
    }
    let (s, _, d) = get_json(
        &c,
        &format!("{base}/api/orderflow/delta?symbol=BTCUSD&feed=mt5&{past}"),
    )
    .await;
    assert_eq!(
        s, 200,
        "a real-source delta never calls history, so feed=mt5 still answers: {d}"
    );

    let logs = c
        .get(format!("{base}/api/logs?since=0"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let warns: Vec<&str> = logs["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["lvl"] == "warn" && l["src"] == "server")
        .filter_map(|l| l["msg"].as_str())
        .filter(|m| m.starts_with("orderflow "))
        .collect();
    assert!(
        warns.contains(&"orderflow delta BTCUSD M2: 'M2'"),
        "{warns:?}"
    );
    assert!(
        warns.contains(
            &"orderflow profile BTCUSD None: mt5 history is not available in this server (D-001)"
        ),
        "{warns:?}"
    );
    assert_eq!(
        warns.len(),
        errors.len(),
        "one warn line per error: {warns:?}"
    );
}
