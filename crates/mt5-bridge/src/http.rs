//! The only axum module: the four GET routes, the bearer gate, and the frozen wire shapes.
//!
//! Parity contract (08-ARCHITECTURE.md rules 1-7), all built here and nowhere else:
//! `/health`, `/symbols`, `/bars?symbol=&tf=&limit=&to=`, `/wait?symbol=&tf=&after=`; a 401
//! `{"error":"unauthorized"}` for anything but the exact `Bearer <token>` header (the oracle
//! compares the whole header value, `hmac.compare_digest`); 502 `{"error":"<text>"}` for every
//! downstream failure; 404 `{"error":"not found"}` for an unknown path.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{middleware, Json, Router};
use serde_json::{json, Value};
use subtle::ConstantTimeEq;

use crate::live::PollerSet;
use crate::term::{account_health, bars, symbols, Bar, Terminal, Terminals, Tf};

/// The `/wait` answer: as long as the bridge's own long-poll plus a margin (architecture
/// decision 4 in the 07 contract; the same 25 s the hub's client uses).
const WAIT_HEADROOM_S: u64 = 5;

/// What the router needs: the three sessions, the poller set, the token, the offset snapshot.
pub struct App<M: Terminal> {
    pub terminals: Terminals<M>,
    pub pollers: Arc<PollerSet<M>>,
    pub token: String,
    pub offset: tokio::sync::watch::Receiver<crate::term::Offset>,
}

impl<M: Terminal> Clone for App<M> {
    fn clone(&self) -> Self {
        Self {
            terminals: Terminals::new(
                self.terminals.live.clone(),
                self.terminals.hist.clone(),
                self.terminals.misc.clone(),
            ),
            pollers: self.pollers.clone(),
            token: self.token.clone(),
            offset: self.offset.clone(),
        }
    }
}

/// Build the router. Every route sits behind the bearer gate, including the 404 fallback (the
/// oracle checks the header before it looks at the path).
pub fn router<M: Terminal + 'static>(app: App<M>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/symbols", get(symbols_route))
        .route("/bars", get(bars_route))
        .route("/wait", get(wait_route))
        .fallback(not_found)
        .layer(middleware::from_fn_with_state(
            Arc::new(app.clone()),
            bearer_gate,
        ))
        .with_state(Arc::new(app))
}

fn error_json(status: StatusCode, text: impl AsRef<str>) -> Response {
    let body = json!({"error": text.as_ref()}).to_string();
    (status, [(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// The gate: the header value must equal `Bearer <token>` byte for byte, compared in constant
/// time so a near miss leaks nothing.
async fn bearer_gate<M: Terminal + 'static>(
    State(app): State<Arc<App<M>>>,
    req: Request,
    next: middleware::Next,
) -> Response {
    let want = format!("Bearer {}", app.token);
    let got = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    // Length is public (the oracle's compare_digest also leaks it); the bytes are not.
    let same = got.len() == want.len() && bool::from(got.as_bytes().ct_eq(want.as_bytes()));
    if !same {
        return error_json(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    next.run(req).await
}

async fn not_found() -> Response {
    error_json(StatusCode::NOT_FOUND, "not found")
}

fn query(req: &Request) -> std::collections::HashMap<String, String> {
    req.uri()
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|pair| {
                    let (k, v) = pair.split_once('=')?;
                    Some((percent_decode(k), percent_decode(v)))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `+` is a space and `%XX` a byte (the oracle uses `parse_qs`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

async fn health<M: Terminal + 'static>(State(app): State<Arc<App<M>>>) -> Response {
    let account = account_health(&*app.terminals.misc).await;
    let server = match account {
        Ok(s) => s,
        Err(e) => return error_json(StatusCode::BAD_GATEWAY, e.to_string()),
    };
    let off = app.offset.borrow().clone();
    // Key order is part of the contract; `json!` preserves insertion order only with the
    // preserve_order feature (a dev-only feature in this workspace), so the body is built by hand.
    let body = format!(
        r#"{{"ok":true,"server":{},"pollers":{},"offset_s":{},"offset_src":{},"measured_at":{}}}"#,
        serde_json::to_string(&server).unwrap_or_else(|_| "null".to_string()),
        app.pollers.len(),
        off.offset_s,
        serde_json::to_string(&off.offset_src).unwrap_or_else(|_| "null".to_string()),
        off.measured_at
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

async fn symbols_route<M: Terminal + 'static>(State(app): State<Arc<App<M>>>) -> Response {
    match symbols(&*app.terminals.misc).await {
        Ok(rows) => Json(Value::Array(rows)).into_response(),
        Err(e) => error_json(StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

async fn bars_route<M: Terminal + 'static>(
    State(app): State<Arc<App<M>>>,
    req: Request,
) -> Response {
    let q = query(&req);
    let Some(symbol) = q.get("symbol") else {
        // The oracle's q["symbol"] raises KeyError; str(KeyError) is the quoted name.
        return error_json(StatusCode::BAD_GATEWAY, "'symbol'");
    };
    let tf_name = q.get("tf").cloned().unwrap_or_else(|| "M1".to_string());
    let tf = match Tf::try_from(tf_name.as_str()) {
        Ok(tf) => tf,
        Err(e) => return error_json(StatusCode::BAD_GATEWAY, e),
    };
    // The oracle hands the raw strings to int(), which rejects fractional and non-digit text
    // with this exact message.
    let int_field = |raw: &str| -> Result<i64, String> {
        raw.trim()
            .parse::<i64>()
            .map_err(|_| format!("invalid literal for int() with base 10: '{}'", raw.trim()))
    };
    let limit = match q.get("limit") {
        Some(l) => match int_field(l) {
            Ok(v) => v,
            Err(e) => return error_json(StatusCode::BAD_GATEWAY, e),
        },
        None => 1000,
    };
    let to_ms = match q.get("to") {
        Some(t) if !t.is_empty() => match int_field(t) {
            Ok(v) => Some(v),
            Err(e) => return error_json(StatusCode::BAD_GATEWAY, e),
        },
        _ => None,
    };
    let off = app.offset.borrow().offset_s;
    match bars(
        &*app.terminals.hist,
        symbol,
        tf,
        limit,
        to_ms,
        off,
        &crate::term::SyncPolicy::default(),
    )
    .await
    {
        // The body must be a bare array (rule 3); Json serializes a Vec<Bar> exactly so.
        Ok(rows) => Json(rows).into_response(),
        Err(e) => error_json(StatusCode::BAD_GATEWAY, e.to_string()),
    }
}

async fn wait_route<M: Terminal + 'static>(
    State(app): State<Arc<App<M>>>,
    req: Request,
) -> Response {
    let q = query(&req);
    let Some(symbol) = q.get("symbol") else {
        return error_json(StatusCode::BAD_GATEWAY, "'symbol'");
    };
    let tf_name = q.get("tf").cloned().unwrap_or_else(|| "M1".to_string());
    let tf = match Tf::try_from(tf_name.as_str()) {
        Ok(tf) => tf,
        Err(e) => return error_json(StatusCode::BAD_GATEWAY, e),
    };
    let after = q.get("after").cloned().unwrap_or_default();
    let deadline = Duration::from_secs(crate::live::WAIT_S + WAIT_HEADROOM_S);
    let answered = tokio::time::timeout(deadline, app.pollers.wait(symbol, tf, &after)).await;
    let snap = match answered {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return error_json(StatusCode::BAD_GATEWAY, &e),
        Err(_) => return error_json(StatusCode::BAD_GATEWAY, "the wait deadline expired"),
    };
    let body = wait_body(&snap);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

/// `{"bar": <bar|null>, "key": "<key>", "tick_ms": <int|null>}` in that key order.
fn wait_body(snap: &crate::live::LiveSnapshot) -> String {
    let bar = match snap.bar.as_ref() {
        Some(b) => bar_json(b),
        None => "null".to_string(),
    };
    let tick = match snap.tick_ms {
        Some(ms) => ms.to_string(),
        None => "null".to_string(),
    };
    format!(
        r#"{{"bar":{},"key":{},"tick_ms":{}}}"#,
        bar,
        serde_json::to_string(&snap.key).unwrap_or_else(|_| "\"\"".to_string()),
        tick
    )
}

fn bar_json(b: &Bar) -> String {
    format!(
        r#"{{"time":{},"open":{},"high":{},"low":{},"close":{},"volume":{}}}"#,
        b.time,
        crate::term::py_str(b.open),
        crate::term::py_str(b.high),
        crate::term::py_str(b.low),
        crate::term::py_str(b.close),
        crate::term::py_str(b.volume)
    )
}

/// Serve helper so tests and `main` share one call.
pub async fn serve<M: Terminal + 'static>(
    listener: tokio::net::TcpListener,
    app: App<M>,
) -> std::io::Result<()> {
    axum::serve(listener, router(app)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering as AtomicOrdering};

    use std::time::Duration as StdDuration;

    use axum::body::Body;
    use tower::ServiceExt;

    use tokio::sync::watch;

    use crate::live::LivePolicy;
    use crate::term::Offset;

    /// An in-memory terminal whose newest bar advances one minute per `get_chart_history` call.
    /// `fail_history` turns history reads into transport errors (AC-4).
    struct Rt {
        calls: AtomicUsize,
        fail_history: AtomicBool,
    }

    impl Rt {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                fail_history: AtomicBool::new(false),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(AtomicOrdering::SeqCst)
        }
    }

    impl Terminal for Rt {
        fn call(
            &self,
            name: &str,
            _args: Value,
        ) -> impl std::future::Future<Output = Result<Value, crate::mcp::McpError>> + Send {
            let n = self.calls.fetch_add(1, AtomicOrdering::SeqCst) as i64;
            let fail = self.fail_history.load(AtomicOrdering::SeqCst);
            async move {
                match name {
                    "get_chart_history" if fail => {
                        Err(crate::mcp::McpError::Transport("terminal is down".into()))
                    }
                    "get_chart_history" => Ok(json!({"history": [{
                        "time": crate::term::iso(1_764_150_000 + n * 60),
                        "open": 1.0, "high": 2.0, "low": 0.5, "close": 1.5,
                        "tick_volume": 7.0}]})),
                    "get_chart_ticks_history" => Ok(json!({"history": []})),
                    "get_marketwatch_symbols" => Ok(json!({"symbols": [
                        {"symbol": "EURUSD", "description": "Euro vs US Dollar",
                         "digits": 5, "category": "majors"},
                        {"symbol": "HB", "digits": 0, "category": "heartbeat"}]})),
                    "get_trading_account_info" => Ok(json!({"account": {
                        "balance": 1000.0, "equity": 1000.0, "server": "Exness-MT5Real28"}})),
                    other => Err(crate::mcp::McpError::Terminal(format!("no tool {other}"))),
                }
            }
        }
    }

    async fn app_with(rt: Arc<Rt>) -> Router {
        let (offset_tx, offset_rx) = watch::channel(Offset {
            offset_s: 10800,
            offset_src: "terminal server time".to_string(),
            measured_at: 1_764_150_000,
        });
        std::mem::forget(offset_tx);
        let pollers = Arc::new(PollerSet::new(
            rt.clone(),
            offset_rx.clone(),
            LivePolicy {
                poll: StdDuration::from_millis(10),
                idle: StdDuration::from_millis(500),
                wait: StdDuration::from_millis(400),
            },
        ));
        let app = App {
            terminals: Terminals::new(rt.clone(), rt.clone(), rt.clone()),
            pollers,
            token: "sekret".to_string(),
            offset: offset_rx,
        };
        router(app)
    }

    async fn get(router: &Router, path: &str, auth: Option<&str>) -> (StatusCode, String) {
        let req = Request::builder()
            .method("GET")
            .uri(path)
            .header(header::AUTHORIZATION, auth.unwrap_or(""))
            .body(Body::empty())
            .expect("a request");
        let res = router.clone().oneshot(req).await.expect("a response");
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
            .await
            .expect("a body");
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    #[tokio::test]
    async fn a_missing_or_near_miss_bearer_is_401_unauthorized() {
        let app = app_with(Rt::new()).await;
        let (code, body) = get(&app, "/health", None).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
        assert_eq!(body, r#"{"error":"unauthorized"}"#);
        let (code, body) = get(&app, "/health", Some("Bearer toke")).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "near miss must not pass");
        assert_eq!(body, r#"{"error":"unauthorized"}"#);
        let (code, body) = get(&app, "/health", Some("bearer sekret")).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "scheme case matters");
        assert_eq!(body, r#"{"error":"unauthorized"}"#);
        // The 404 fallback is behind the gate too (the oracle checks the header first).
        let (code, _) = get(&app, "/nope", None).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn health_carries_the_contract_key_order() {
        let app = app_with(Rt::new()).await;
        let (code, body) = get(&app, "/health", Some("Bearer sekret")).await;
        assert_eq!(code, StatusCode::OK);
        // Key order is contract rule 5; preserve_order keeps json! literal order.
        assert_eq!(
            body,
            r#"{"ok":true,"server":"Exness-MT5Real28","pollers":0,"offset_s":10800,"offset_src":"terminal server time","measured_at":1764150000}"#
        );
    }

    #[tokio::test]
    async fn bars_answers_a_bare_array_in_bar_field_order() {
        let app = app_with(Rt::new()).await;
        let (code, body) = get(
            &app,
            "/bars?symbol=EURUSD&tf=M1&limit=5",
            Some("Bearer sekret"),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let rows: Vec<Value> = serde_json::from_str(&body).expect("a bare array");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        // Field order (rule 3) verbatim; the newest row is one of the fake's grid bars
        // (server-time base + n*60) with the 3 h offset subtracted, so assert the shape and the
        // shift rather than which pass of the resync loop produced it.
        let want_time = row["time"].to_string();
        let want = format!(
            r#"{{"time":{want_time},"open":1.0,"high":2.0,"low":0.5,"close":1.5,"volume":7.0}}"#
        );
        assert_eq!(
            serde_json::to_string(row).unwrap(),
            want,
            "bare-array row in contract field order (rule 3)"
        );
        let shifted = row["time"].as_i64().unwrap() + 10_800;
        assert!(
            shifted >= 1_764_150_000 && (shifted - 1_764_150_000) % 60 == 0,
            "time = grid bar minus offset, got {}",
            row["time"]
        );
        let (code, _) = get(&app, "/bars?symbol=EURUSD", Some("Bearer sekret")).await;
        assert_eq!(code, StatusCode::OK, "the default limit and tf");
        let (code, body) = get(&app, "/bars?symbol=EURUSD&tf=M2", Some("Bearer sekret")).await;
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        assert_eq!(body, r#"{"error":"unknown timeframe M2"}"#);
    }

    #[tokio::test]
    async fn wait_answers_snapshot_key_tick_in_contract_order() {
        let app = app_with(Rt::new()).await;
        let (code, body) = get(
            &app,
            "/wait?symbol=EURUSD&tf=M1&after=",
            Some("Bearer sekret"),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(
            body,
            r#"{"bar":{"time":1764139200,"open":1.0,"high":2.0,"low":0.5,"close":1.5,"volume":7.0},"key":"1764139200,1.5,2.0,0.5,7.0","tick_ms":null}"#
        );
    }

    #[tokio::test]
    async fn unknown_path_and_downstream_errors_map_to_the_contract_shapes() {
        let rt = Rt::new();
        let app = app_with(rt.clone()).await;
        let (code, body) = get(&app, "/nope", Some("Bearer sekret")).await;
        assert_eq!(code, StatusCode::NOT_FOUND);
        assert_eq!(body, r#"{"error":"not found"}"#);
        rt.fail_history.store(true, AtomicOrdering::SeqCst);
        let (code, body) = get(&app, "/bars?symbol=EURUSD", Some("Bearer sekret")).await;
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        assert_eq!(body, r#"{"error":"terminal is down"}"#);
        let (code, body) = get(&app, "/symbols", Some("Bearer sekret")).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(
            body,
            r#"[{"name":"EURUSD","desc":"Euro vs US Dollar","digits":5,"path":"Forex"}]"#
        );
    }

    #[tokio::test]
    async fn one_poller_serves_two_concurrent_waits() {
        let rt = Rt::new();
        let app = app_with(rt.clone()).await;
        let before = rt.calls();
        let (_, first) = get(
            &app,
            "/wait?symbol=EURUSD&tf=M1&after=",
            Some("Bearer sekret"),
        )
        .await;
        assert!(!first.is_empty(), "the first wait answered");
        let key: Value = serde_json::from_str(&first).expect("json");
        let key = key["key"].as_str().unwrap().to_string();
        let b = get(
            &app,
            &format!("/wait?symbol=EURUSD&tf=M1&after={key}"),
            Some("Bearer sekret"),
        )
        .await;
        assert_eq!(b.0, StatusCode::OK, "{}", b.1);
        assert!(
            rt.calls() - before < 40,
            "one shared poller made {} calls",
            rt.calls() - before
        );
    }

    #[tokio::test]
    async fn a_waiter_on_an_unchanged_key_returns_by_the_deadline() {
        let app = app_with(Rt::new()).await;
        let (_, first) = get(
            &app,
            "/wait?symbol=EURUSD&tf=M1&after=",
            Some("Bearer sekret"),
        )
        .await;
        let body: Value = serde_json::from_str(&first).expect("json");
        let key = body["key"].as_str().unwrap().to_string();
        let started = std::time::Instant::now();
        let (code, second) = get(
            &app,
            &format!("/wait?symbol=EURUSD&tf=M1&after={key}"),
            Some("Bearer sekret"),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
        let body: Value = serde_json::from_str(&second).expect("json");
        // This fake advances its newest bar every read, so the waiter is woken by a change
        // before the deadline; the unchanged-key-to-deadline case is live.rs's
        // a_quiet_market_returns_at_the_deadline.
        assert_ne!(
            body["key"].as_str(),
            Some(key.as_str()),
            "the bar moved under the waiter"
        );
        assert!(started.elapsed() < StdDuration::from_secs(2), "prompt wake");
    }

    #[tokio::test]
    async fn a_poller_error_surfaces_as_502_with_the_text() {
        let rt = Rt::new();
        let app = app_with(rt.clone()).await;
        rt.fail_history.store(true, AtomicOrdering::SeqCst);
        let (code, body) = get(
            &app,
            "/wait?symbol=EURUSD&tf=M1&after=",
            Some("Bearer sekret"),
        )
        .await;
        assert_eq!(code, StatusCode::BAD_GATEWAY);
        assert_eq!(body, r#"{"error":"terminal is down"}"#);
        // A later wait recovers: the poller restarts on the next entry.
        rt.fail_history.store(false, AtomicOrdering::SeqCst);
        tokio::time::sleep(StdDuration::from_millis(80)).await;
        let (code, _) = get(
            &app,
            "/wait?symbol=EURUSD&tf=M1&after=",
            Some("Bearer sekret"),
        )
        .await;
        assert_eq!(code, StatusCode::OK);
    }

    #[tokio::test]
    async fn an_idle_entry_disappears_from_the_health_count() {
        let app = app_with(Rt::new()).await;
        let _ = get(
            &app,
            "/wait?symbol=EURUSD&tf=M1&after=",
            Some("Bearer sekret"),
        )
        .await;
        let (_, body) = get(&app, "/health", Some("Bearer sekret")).await;
        assert!(body.contains(r#""pollers":1"#), "{body}");
        tokio::time::sleep(StdDuration::from_millis(800)).await;
        let (_, body) = get(&app, "/health", Some("Bearer sekret")).await;
        assert!(
            body.contains(r#""pollers":0"#),
            "the reaper stopped it: {body}"
        );
    }

    #[test]
    fn percent_decode_is_parse_qs() {
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%20"), " ");
        assert_eq!(percent_decode("1764150000%2C1.5"), "1764150000,1.5");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("%zz"), "%zz", "an invalid escape is literal");
        let _ = AtomicI64::new(0);
    }
}
