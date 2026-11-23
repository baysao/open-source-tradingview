//! AC-4 of 01-02: `/ws` end to end against a local fake Bybit (REST klines plus a websocket that
//! replays the recorded `publicTrade` fixture and drops its first connection), no internet.
//!
//! Script: subscribe Bybit M1, get `history` and `bar` frames; resubscribe M5 on the same socket
//! (replaces the stream); the fake drops the upstream, the viewer gets `status "Bybit
//! reconnecting: ..."` and then bars again; `{"tf":"M2"}` gets the error and the socket stays open.

use axum::extract::ws::{Message as AxMessage, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use hub_core::config::Config;
use hub_core::sources::Endpoints;
use hub_core::AppState;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone)]
struct Fake {
    frames: Arc<Vec<String>>,
    /// First trade time, epoch seconds; the kline seed is the bucket before it, per interval.
    first_s: i64,
    connections: Arc<AtomicUsize>,
}

fn fixture() -> Vec<String> {
    let p =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bybit_publictrade.jsonl");
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

async fn kline(
    State(f): State<Fake>,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let span: i64 = match q.get("interval").map(String::as_str) {
        Some("D") => 86_400,
        Some(m) => m.parse::<i64>().unwrap_or(1) * 60,
        None => 60,
    };
    let seed_ms = (f.first_s - f.first_s % span - span) * 1000;
    let row = vec![
        seed_ms.to_string(),
        "1".into(),
        "1".into(),
        "1".into(),
        "1".into(),
        "1".into(),
        "1".into(),
    ];
    axum::Json(serde_json::json!({"retCode": 0, "retMsg": "OK", "result": {"list": [row]}}))
}

async fn upstream(State(f): State<Fake>, ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(move |mut socket| async move {
        let n = f.connections.fetch_add(1, Ordering::SeqCst);
        // wait for the subscribe request
        let _ = socket.recv().await;
        let limit = if n == 0 { 60 } else { usize::MAX };
        loop {
            for (i, fr) in f.frames.iter().enumerate() {
                if i >= limit {
                    let _ = socket.send(AxMessage::Close(None)).await;
                    return;
                }
                if socket
                    .send(AxMessage::Text(fr.clone().into()))
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    })
}

async fn start_fake() -> (String, Arc<AtomicUsize>) {
    let frames = fixture();
    let first: Value = serde_json::from_str(&frames[0]).unwrap();
    let t = first["data"][0]["T"].as_i64().unwrap() / 1000;
    let fake = Fake {
        frames: Arc::new(frames),
        first_s: t,
        connections: Arc::new(AtomicUsize::new(0)),
    };
    let conns = fake.connections.clone();
    let app = Router::new()
        .route("/v5/market/kline", get(kline))
        .route("/ws/{cat}", get(upstream))
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("127.0.0.1:{}", addr.port()), conns)
}

async fn start_hub(fake: &str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = Config::from_lookup(|_| None).unwrap();
    cfg.http_port = port;
    let ep = Endpoints {
        binance_rest: "http://127.0.0.1:9".into(),
        bybit_rest: format!("http://{fake}"),
        deriv_ws: "ws://127.0.0.1:9".into(),
        binance_ws: "ws://127.0.0.1:9".into(),
        bybit_ws: format!("ws://{fake}/ws/"),
    };
    let state = AppState::build_with(cfg, ep);
    tokio::spawn(async move { hub_core::serve(listener, state).await.unwrap() });
    format!("ws://127.0.0.1:{port}/ws")
}

type Client =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Reads frames until `pred` matches one, or fails after `within`.
async fn until(
    ws: &mut Client,
    within: Duration,
    what: &str,
    pred: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let v: Value = serde_json::from_str(&t).unwrap();
                if pred(&v) {
                    return v;
                }
            }
            Ok(Some(Ok(_))) => continue,
            other => panic!("waiting for {what}: {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bybit_end_to_end_against_fakes() {
    let (fake, conns) = start_fake().await;
    let url = start_hub(&fake).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let send = |s: &'static str| Message::Text(s.into());

    ws.send(send(
        r#"{"source":"bybit","symbol":"BYBIT_BTCUSDT","tf":"M1"}"#,
    ))
    .await
    .unwrap();
    let h = until(&mut ws, Duration::from_secs(10), "history M1", |v| {
        v["type"] == "history"
    })
    .await;
    assert_eq!(
        (h["source"].as_str(), h["symbol"].as_str(), h["tf"].as_str()),
        (Some("bybit"), Some("BYBIT_BTCUSDT"), Some("M1"))
    );
    assert_eq!(h["bars"].as_array().map(Vec::len), Some(1));
    let b = until(&mut ws, Duration::from_secs(10), "bar M1", |v| {
        v["type"] == "bar"
    })
    .await;
    for k in ["bar", "sent", "trade_ms", "lag_ms"] {
        assert!(b.get(k).is_some(), "bar frame has {k}: {b}");
    }

    ws.send(send(
        r#"{"source":"bybit","symbol":"BYBIT_BTCUSDT","tf":"M5"}"#,
    ))
    .await
    .unwrap();
    until(&mut ws, Duration::from_secs(10), "history M5", |v| {
        v["type"] == "history" && v["tf"] == "M5"
    })
    .await;
    let st = until(&mut ws, Duration::from_secs(10), "reconnect status", |v| {
        v["type"] == "status"
    })
    .await;
    assert!(
        st["msg"]
            .as_str()
            .unwrap()
            .starts_with("Bybit reconnecting: "),
        "{st}"
    );
    until(
        &mut ws,
        Duration::from_secs(10),
        "bars after reconnect",
        |v| v["type"] == "bar",
    )
    .await;
    assert!(
        conns.load(Ordering::SeqCst) >= 2,
        "the hub reconnected upstream"
    );
    assert_eq!(
        conns.load(Ordering::SeqCst),
        2,
        "one upstream for M1 and M5 of one symbol, plus the reconnect"
    );

    ws.send(send(r#"{"tf":"M2"}"#)).await.unwrap();
    let e = until(&mut ws, Duration::from_secs(5), "tf error", |v| {
        v["type"] == "error"
    })
    .await;
    assert_eq!(e["msg"], "Unknown timeframe M2");
    ws.send(send(
        r#"{"source":"bybit","symbol":"BYBIT_BTCUSDT","tf":"M1"}"#,
    ))
    .await
    .unwrap();
    until(
        &mut ws,
        Duration::from_secs(10),
        "history after error",
        |v| v["type"] == "history",
    )
    .await;

    ws.send(Message::Text("not json".into())).await.unwrap();
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert_eq!(
        closed,
        Ok(true),
        "a non-JSON message closes the socket (Python parity)"
    );
}
