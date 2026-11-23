//! G1 contract suite: the Python server is the oracle.
//!
//! For every case in `tests/contract/requests.toml`, the same request goes to the Python server
//! (`CONTRACT_PY_URL`) and the Rust server (`CONTRACT_RS_URL`, or this crate's router served
//! in-process when unset). Both must return the same HTTP status and the same JSON shape: the same
//! key set at every object level and the same JSON type per key. Values are ignored unless the case
//! names a check. Nothing passes silently: an unset `CONTRACT_PY_URL` is reported as skipped, and a
//! case whose upstream fails on both servers is reported `skipped (upstream)`.
//!
//! Run: `python hub/server.py` (port 8770), then
//! `CONTRACT_PY_URL=http://127.0.0.1:8770 cargo test -p hub-core --test contract -- --nocapture`.

use hub_core::config::Config;
use hub_core::AppState;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Cases {
    allowed: Allowed,
    #[serde(default)]
    case: Vec<Case>,
    #[serde(default)]
    probe: Vec<Probe>,
}

#[derive(Deserialize)]
struct Allowed {
    paths: Vec<String>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    path: String,
    status: u16,
    #[serde(default)]
    bars_len: Option<usize>,
    #[serde(default)]
    error: Option<String>,
    /// `POST` sends `body` (or `body_bytes` filler bytes) with `content_type` and `origin`.
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    body_bytes: Option<usize>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    origin: Option<String>,
    /// Top-level keys that must be absent on both servers.
    #[serde(default)]
    absent: Vec<String>,
}

#[derive(Deserialize)]
struct Probe {
    path: String,
    status: u16,
    #[serde(default)]
    method: Option<String>,
}

enum Outcome {
    Passed,
    Skipped(String),
    Failed(String),
}

/// The JSON shape of a value at `path`: objects keep their keys, arrays the set of their
/// element shapes, scalars their JSON type. Paths in `allowed` collapse to `*`.
fn shape(v: &Value, path: &str, allowed: &BTreeSet<String>) -> String {
    if allowed.contains(path) {
        return "*".into();
    }
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) => "bool".into(),
        Value::Number(_) => "number".into(),
        Value::String(_) => "string".into(),
        Value::Array(items) => {
            let child = format!("{path}[]");
            let set: BTreeSet<String> = items.iter().map(|i| shape(i, &child, allowed)).collect();
            format!("[{}]", set.into_iter().collect::<Vec<_>>().join("|"))
        }
        Value::Object(map) => {
            let fields: BTreeMap<&String, String> = map
                .iter()
                .map(|(k, val)| {
                    let child = if path.is_empty() {
                        k.clone()
                    } else {
                        format!("{path}.{k}")
                    };
                    (k, shape(val, &child, allowed))
                })
                .collect();
            let inner: Vec<String> = fields
                .into_iter()
                .map(|(k, s)| format!("{k}:{s}"))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// The first point where two shapes differ, for a readable failure.
fn first_difference(a: &Value, b: &Value, path: &str, allowed: &BTreeSet<String>) -> String {
    match (a, b) {
        (Value::Object(ma), Value::Object(mb)) => {
            let ka: BTreeSet<&String> = ma.keys().collect();
            let kb: BTreeSet<&String> = mb.keys().collect();
            if ka != kb {
                let only_py: Vec<_> = ka.difference(&kb).collect();
                let only_rs: Vec<_> = kb.difference(&ka).collect();
                return format!(
                    "at {path:?}: keys only in Python {only_py:?}, only in Rust {only_rs:?}"
                );
            }
            for k in ka {
                let child = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                if shape(&ma[k], &child, allowed) != shape(&mb[k], &child, allowed) {
                    return first_difference(&ma[k], &mb[k], &child, allowed);
                }
            }
            format!("at {path:?}: shapes differ")
        }
        _ => format!(
            "at {path:?}: Python {} against Rust {}",
            shape(a, path, allowed),
            shape(b, path, allowed)
        ),
    }
}

async fn fetch(client: &reqwest::Client, base: &str, path: &str) -> Result<(u16, Value), String> {
    send(client.get(format!("{base}{path}"))).await
}

/// A case's request: GET, or POST with its body, content type and origin.
async fn fetch_case(
    client: &reqwest::Client,
    base: &str,
    case: &Case,
) -> Result<(u16, Value), String> {
    if case.method.as_deref() != Some("POST") {
        return fetch(client, base, &case.path).await;
    }
    let body = match (case.body_bytes, &case.body) {
        (Some(n), _) => vec![b' '; n],
        (None, Some(b)) => b.clone().into_bytes(),
        (None, None) => Vec::new(),
    };
    let mut req = client.post(format!("{base}{}", case.path)).body(body);
    if let Some(ct) = &case.content_type {
        req = req.header("Content-Type", ct);
    }
    if let Some(o) = &case.origin {
        req = req.header("Origin", o);
    }
    send(req).await
}

async fn send(req: reqwest::RequestBuilder) -> Result<(u16, Value), String> {
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    let json = serde_json::from_str(&text).unwrap_or(Value::String(format!(
        "<non-JSON body, {} bytes>",
        text.len()
    )));
    Ok((status, json))
}

async fn wait_ready(client: &reqwest::Client, base: &str, deadline: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < deadline {
        if let Ok((200, v)) = fetch(client, base, "/api/status").await {
            if v["deriv"]["ok"] == Value::Bool(true) && v["bybit"]["ok"] == Value::Bool(true) {
                return true;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    false
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Serves the router in-process on an ephemeral port; returns its base URL and the guards.
async fn start_in_process() -> (
    String,
    hub_core::BackgroundTasks,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    let static_dir = repo_root().join("hub");
    let mut cfg = Config::from_lookup(|_| None).expect("default config");
    cfg.http_port = port;
    cfg.static_dir = static_dir;
    // real upstreams, log kept in memory (no hub.log written by the suite)
    let state = AppState::build_with(cfg, hub_core::sources::Endpoints::default());
    let bg = state.spawn_background();
    let server = tokio::spawn(async move {
        hub_core::serve(listener, state).await.expect("serve");
    });
    (format!("http://127.0.0.1:{port}"), bg, server)
}

async fn run_case(
    client: &reqwest::Client,
    py: &str,
    rs: &str,
    case: &Case,
    allowed: &BTreeSet<String>,
) -> Outcome {
    let (p, r) = tokio::join!(fetch_case(client, py, case), fetch_case(client, rs, case));
    let (p, r) = match (p, r) {
        (Ok(p), Ok(r)) => (p, r),
        (Err(e), _) => return Outcome::Failed(format!("Python request failed: {e}")),
        (_, Err(e)) => return Outcome::Failed(format!("Rust request failed: {e}")),
    };
    if case.status != 502 && p.0 == 502 && r.0 == 502 {
        return Outcome::Skipped(format!("upstream: Python {} / Rust {}", p.1, r.1));
    }
    if p.0 != r.0 {
        return Outcome::Failed(format!(
            "status Python {} Rust {} (Rust body {})",
            p.0, r.0, r.1
        ));
    }
    if r.0 != case.status {
        return Outcome::Failed(format!(
            "status {} expected {} (body {})",
            r.0, case.status, r.1
        ));
    }
    if shape(&p.1, "", allowed) != shape(&r.1, "", allowed) {
        return Outcome::Failed(first_difference(&p.1, &r.1, "", allowed));
    }
    if let Some(n) = case.bars_len {
        for (who, v) in [("Python", &p.1), ("Rust", &r.1)] {
            let len = v.as_array().map(Vec::len);
            if len != Some(n) {
                return Outcome::Failed(format!("{who} returned {len:?} bars, expected {n}"));
            }
        }
    }
    for key in &case.absent {
        for (who, v) in [("Python", &p.1), ("Rust", &r.1)] {
            if v.get(key).is_some() {
                return Outcome::Failed(format!("{who} returned key {key:?}, expected none"));
            }
        }
    }
    if let Some(want) = &case.error {
        for (who, v) in [("Python", &p.1), ("Rust", &r.1)] {
            if v["error"].as_str() != Some(want.as_str()) {
                return Outcome::Failed(format!("{who} error {} expected {want:?}", v["error"]));
            }
        }
    }
    Outcome::Passed
}

async fn run_probe(client: &reqwest::Client, rs: &str, probe: &Probe) -> Outcome {
    let url = format!("{rs}{}", probe.path);
    let req = match probe.method.as_deref() {
        Some("HEAD") => client.head(&url),
        _ => client.get(&url),
    };
    match req.send().await {
        Ok(resp) if resp.status().as_u16() == probe.status => Outcome::Passed,
        Ok(resp) => Outcome::Failed(format!(
            "status {} expected {}",
            resp.status(),
            probe.status
        )),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn contract() {
    let raw = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract/requests.toml"),
    )
    .expect("read requests.toml");
    let cases: Cases = toml::from_str(&raw).expect("parse requests.toml");
    let allowed: BTreeSet<String> = cases.allowed.paths.iter().cloned().collect();
    let total = cases.case.len() + cases.probe.len();

    let Some(py) = std::env::var("CONTRACT_PY_URL")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        println!("contract: skipped: CONTRACT_PY_URL unset ({total} cases not run)");
        println!("contract: 0 passed, {total} skipped, 0 failed");
        return;
    };
    let py = py.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();

    let mut _guards = None;
    let rs = match std::env::var("CONTRACT_RS_URL")
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => {
            let (url, bg, server) = start_in_process().await;
            println!("contract: Rust router served in-process at {url}");
            _guards = Some((bg, server));
            url
        }
    };
    let (py_ready, rs_ready) = tokio::join!(
        wait_ready(&client, &py, Duration::from_secs(90)),
        wait_ready(&client, &rs, Duration::from_secs(90))
    );
    assert!(
        py_ready,
        "Python server at {py} did not report deriv and bybit ok within 90 s"
    );
    assert!(
        rs_ready,
        "Rust server at {rs} did not report deriv and bybit ok within 90 s"
    );

    let (mut passed, mut skipped, mut failed) = (0, 0, 0);
    for case in &cases.case {
        match run_case(&client, &py, &rs, case, &allowed).await {
            Outcome::Passed => {
                passed += 1;
                println!("  pass  {}", case.name);
            }
            Outcome::Skipped(why) => {
                skipped += 1;
                println!("  skip  {} ({why})", case.name);
            }
            Outcome::Failed(why) => {
                failed += 1;
                println!("  FAIL  {}: {why}", case.name);
            }
        }
    }
    for probe in &cases.probe {
        let label = format!(
            "probe {} {}",
            probe.method.as_deref().unwrap_or("GET"),
            probe.path
        );
        match run_probe(&client, &rs, probe).await {
            Outcome::Passed => {
                passed += 1;
                println!("  pass  {label}");
            }
            Outcome::Skipped(why) => {
                skipped += 1;
                println!("  skip  {label} ({why})");
            }
            Outcome::Failed(why) => {
                failed += 1;
                println!("  FAIL  {label}: {why}");
            }
        }
    }
    println!("contract: {passed} passed, {skipped} skipped, {failed} failed");
    assert_eq!(failed, 0, "contract cases failed");
}

/// `/api/instruments` by value: Python's own `/api/symbols` fed into the Rust builder must give
/// exactly Python's `/api/instruments` (keys, names, groups, variant order, instrument order).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn instruments_match_python_exactly() {
    let Some(py) = std::env::var("CONTRACT_PY_URL")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        println!("instruments: skipped: CONTRACT_PY_URL unset");
        return;
    };
    let py = py.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    assert!(
        wait_ready(&client, &py, Duration::from_secs(90)).await,
        "Python not ready"
    );
    let (_, symbols) = fetch(&client, &py, "/api/symbols").await.expect("symbols");
    let (_, want) = fetch(&client, &py, "/api/instruments")
        .await
        .expect("instruments");
    let parsed: hub_types::market::SymbolsResponse =
        serde_json::from_value(symbols).expect("Python symbols fit the Rust model");
    let by = |s: hub_types::market::Source| -> Vec<_> {
        parsed
            .symbols
            .iter()
            .filter(|e| e.source() == s)
            .cloned()
            .collect()
    };
    use hub_types::market::Source;
    let built = hub_core::market::instruments::build(
        &by(Source::Deriv),
        &by(Source::Bybit),
        &by(Source::Binance),
    );
    let got = serde_json::to_value(&built).unwrap();
    if got != want {
        let (gi, wi) = (
            got["instruments"].as_array().unwrap(),
            want["instruments"].as_array().unwrap(),
        );
        let first = gi.iter().zip(wi).position(|(a, b)| a != b);
        panic!(
            "instruments differ: Rust {} entries, Python {}; first difference at index {first:?}: Rust {} / Python {}",
            gi.len(),
            wi.len(),
            first.map(|i| gi[i].to_string()).unwrap_or_default(),
            first.map(|i| wi[i].to_string()).unwrap_or_default()
        );
    }
    println!(
        "instruments: {} instruments match Python exactly",
        want["instruments"].as_array().map(Vec::len).unwrap_or(0)
    );
}

/// Message types and field sets seen on one websocket subscription within `window`.
async fn ws_shapes(
    url: &str,
    subscribe: &str,
    window: Duration,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    ws.send(Message::Text(subscribe.to_string().into()))
        .await
        .map_err(|e| e.to_string())?;
    let mut seen: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, ws.next()).await {
        let Message::Text(t) = msg else { continue };
        let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
        let ty = v["type"].as_str().unwrap_or("?").to_string();
        let keys: BTreeSet<String> = v
            .as_object()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        seen.entry(ty).or_default().extend(keys);
    }
    Ok(seen)
}

/// `/ws` and `/api/health` against Python: same message types with the same field sets for a
/// Binance BTCUSDT M1 subscription over 20 s, then the same health shape (feed keys compared as a
/// map, since Binance keys use the mapped symbol by design).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ws_and_health_match_python() {
    let Some(py) = std::env::var("CONTRACT_PY_URL")
        .ok()
        .filter(|v| !v.is_empty())
    else {
        println!("ws: skipped: CONTRACT_PY_URL unset");
        return;
    };
    let py = py.trim_end_matches('/').to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let mut _guards = None;
    let rs = match std::env::var("CONTRACT_RS_URL")
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(url) => url.trim_end_matches('/').to_string(),
        None => {
            let (url, bg, server) = start_in_process().await;
            _guards = Some((bg, server));
            url
        }
    };
    assert!(
        wait_ready(&client, &py, Duration::from_secs(90)).await,
        "Python not ready"
    );
    let (_, sym) = fetch(&client, &py, "/api/symbols")
        .await
        .expect("python symbols");
    let py_ws_port = sym["ws_port"].as_u64().expect("python ws_port");
    let py_host = py
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap_or("127.0.0.1")
        .to_string();
    let py_ws = format!("ws://{py_host}:{py_ws_port}");
    let rs_ws = format!("{}/ws", rs.replacen("http://", "ws://", 1));
    let sub = r#"{"source":"binance","symbol":"BTCUSDT","tf":"M1"}"#;
    let window = Duration::from_secs(20);
    let (p, r) = tokio::join!(
        ws_shapes(&py_ws, sub, window),
        ws_shapes(&rs_ws, sub, window)
    );
    let (p, r) = (p.expect("python ws"), r.expect("rust ws"));
    println!("ws: Python {p:?}");
    println!("ws: Rust   {r:?}");
    assert!(
        p.contains_key("history") && p.contains_key("bar"),
        "Python sent history and bars"
    );
    assert_eq!(p, r, "message types and field sets differ");

    let (ps, pv) = fetch(&client, &py, "/api/health")
        .await
        .expect("python health");
    let (rs_status, rv) = fetch(&client, &rs, "/api/health")
        .await
        .expect("rust health");
    assert_eq!((ps, rs_status), (200, 200));
    let top = |v: &Value| {
        v.as_object()
            .map(|m| m.keys().cloned().collect::<BTreeSet<_>>())
            .unwrap_or_default()
    };
    assert_eq!(top(&pv), top(&rv), "health top-level keys");
    let entry_keys = |v: &Value| -> BTreeSet<String> {
        v["feeds"]
            .as_object()
            .map(|m| {
                m.values()
                    .flat_map(|e| {
                        e.as_object()
                            .map(|o| o.keys().cloned().collect::<Vec<_>>())
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    assert!(
        !entry_keys(&rv).is_empty(),
        "Rust health lists the BTCUSDT upstream"
    );
    assert_eq!(entry_keys(&pv), entry_keys(&rv), "health entry keys");
    println!("health: keys match {:?}", entry_keys(&rv));
}
