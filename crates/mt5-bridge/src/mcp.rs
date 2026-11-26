//! The sealed MCP leaf: JSON-RPC 2.0 over HTTP to the MT5 terminal's built-in MCP server.
//!
//! **Rule: no sibling imports.** This module returns [`serde_json::Value`] and nothing else; the
//! JSON-RPC envelopes, the session header and the SSE scan never leave it.
//!
//! Ported from the oracle `bridge/mt5_bridge.py` (`class Mcp`, lines 53-101): one lock-protected
//! session, the `initialize` + `notifications/initialized` handshake, `Mcp-Session-Id` captured
//! from the response and resent on every later call, the body scanned for `data:` lines (the LAST
//! one wins), `content[].text` joined and parsed, `isError` surfaced as the error text verbatim,
//! and one transport, HTTP-status or parse failure reopening the session ONCE and retrying.

use std::fmt;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::Mutex;

/// One terminal call budget (the oracle's `CALL_TIMEOUT_S`).
const CALL_TIMEOUT_S: u64 = 15;
/// The handshake constants the oracle sends; the terminal keys its session on them.
const PROTOCOL_VERSION: &str = "2025-06-18";
const CLIENT_NAME: &str = "chart-bridge";
const CLIENT_VERSION: &str = "1";

/// Every way a terminal call can fail.
#[derive(Debug)]
pub enum McpError {
    /// A network or HTTP-status failure: the session may be dead (reopens once and retries).
    Transport(String),
    /// The body, or the tool's `content` text, was not JSON (reopens once and retries).
    Parse(String),
    /// The terminal answered `isError`: the text is the oracle's `content` join, verbatim.
    Terminal(String),
    /// The configuration is unusable (no MCP token); the oracle raises and does not retry.
    Config(String),
}

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McpError::Transport(s)
            | McpError::Parse(s)
            | McpError::Terminal(s)
            | McpError::Config(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for McpError {}

struct Session {
    id: Option<String>,
    next: u64,
}

/// One MCP session behind a lock. A dead session (terminal restarted) is reopened once per call.
pub struct Mcp {
    http: reqwest::Client,
    url: String,
    token: String,
    session: Mutex<Session>,
}

impl Mcp {
    /// Build a client for one terminal endpoint. `token` is the `MT5_MCP_TOKEN`; an empty or
    /// blank token makes every call fail the way the oracle does.
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Result<Mcp, McpError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(CALL_TIMEOUT_S))
            .build()
            .map_err(|e| McpError::Config(format!("the HTTP client did not build: {e}")))?;
        Ok(Mcp {
            http,
            url: url.into(),
            token: token.into().trim().to_string(),
            session: Mutex::new(Session { id: None, next: 0 }),
        })
    }

    /// Call one tool: `tools/call` with `args`, the parsed JSON of the answer's text.
    pub async fn call(&self, name: &str, args: Value) -> Result<Value, McpError> {
        let mut s = self.session.lock().await;
        match self.attempt(&mut s, name, &args).await {
            Ok(v) => Ok(v),
            Err(McpError::Transport(_) | McpError::Parse(_)) => {
                self.open(&mut s).await?;
                self.tool_call(&mut s, name, &args).await
            }
            Err(e) => Err(e),
        }
    }

    /// The oracle's first attempt: open if there is no session, then call.
    async fn attempt(&self, s: &mut Session, name: &str, args: &Value) -> Result<Value, McpError> {
        if s.id.is_none() {
            self.open(s).await?;
        }
        self.tool_call(s, name, args).await
    }

    async fn open(&self, s: &mut Session) -> Result<(), McpError> {
        s.id = None;
        s.next += 1;
        let id = s.next;
        self.post(
            s,
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "initialize",
                "params": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": CLIENT_NAME, "version": CLIENT_VERSION},
                },
            }),
        )
        .await?;
        self.post(
            s,
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .await?;
        Ok(())
    }

    async fn tool_call(
        &self,
        s: &mut Session,
        name: &str,
        args: &Value,
    ) -> Result<Value, McpError> {
        s.next += 1;
        let id = s.next;
        let r = self
            .post(
                s,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": "tools/call",
                    "params": {"name": name, "arguments": args},
                }),
            )
            .await?;
        let res = r.get("result").cloned().unwrap_or_else(|| json!({}));
        let txt: String = res
            .get("content")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|c| c.get("text").and_then(Value::as_str))
                    .collect()
            })
            .unwrap_or_default();
        if res.get("isError").and_then(Value::as_bool).unwrap_or(false) {
            return Err(McpError::Terminal(if txt.is_empty() {
                "terminal error".to_string()
            } else {
                txt
            }));
        }
        serde_json::from_str(&txt)
            .map_err(|e| McpError::Parse(format!("the tool answer was not JSON: {e}")))
    }

    /// One POST. Captures the session id, then keeps the last `data:` line of the body (the
    /// oracle's SSE scan); an empty body is `{}`.
    async fn post(&self, s: &mut Session, body: Value) -> Result<Value, McpError> {
        if self.token.is_empty() {
            return Err(McpError::Config("MT5_MCP_TOKEN is not set".to_string()));
        }
        let mut req = self
            .http
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("Authorization", format!("Bearer {}", self.token))
            .body(body.to_string());
        if let Some(id) = &s.id {
            req = req.header("Mcp-Session-Id", id);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(McpError::Transport(format!(
                "the terminal answered {}",
                resp.status()
            )));
        }
        if let Some(id) = resp
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
        {
            s.id = Some(id.to_string());
        }
        let raw = resp
            .text()
            .await
            .map_err(|e| McpError::Transport(e.to_string()))?;
        let mut text = raw.as_str();
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                text = rest.trim();
            }
        }
        if text.trim().is_empty() {
            return Ok(json!({}));
        }
        serde_json::from_str(text).map_err(|e| McpError::Parse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    #[derive(Clone, Debug)]
    struct Seen {
        headers: Vec<(String, String)>,
        body: String,
    }

    impl Seen {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }

        fn json(&self) -> Value {
            serde_json::from_str(&self.body).expect("a JSON request body")
        }
    }

    /// `(status, session id header, body)`. A status of 0 drops the connection unanswered.
    type Answer = (u16, Option<String>, String);

    struct Fake {
        url: String,
        seen: Arc<StdMutex<Vec<Seen>>>,
        _task: tokio::task::JoinHandle<()>,
    }

    impl Fake {
        async fn spawn(script: impl Fn(usize, &Seen) -> Answer + Send + Sync + 'static) -> Fake {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let port = listener.local_addr().unwrap().port();
            let seen = Arc::new(StdMutex::new(Vec::new()));
            let script = Arc::new(script);
            let log = seen.clone();
            let task = tokio::spawn(async move {
                loop {
                    let Ok((mut stream, _)) = listener.accept().await else {
                        continue;
                    };
                    let log = log.clone();
                    let script = script.clone();
                    tokio::spawn(async move {
                        let Some(req) = read_request(&mut stream).await else {
                            return;
                        };
                        let n = {
                            let mut g = log.lock().unwrap();
                            g.push(req.clone());
                            g.len() - 1
                        };
                        let (status, session, body) = script(n, &req);
                        if status == 0 {
                            return;
                        }
                        let mut head = format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n",
                            body.len()
                        );
                        if let Some(id) = session {
                            head.push_str(&format!("Mcp-Session-Id: {id}\r\n"));
                        }
                        head.push_str("\r\n");
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(body.as_bytes()).await;
                        let _ = stream.flush().await;
                    });
                }
            });
            Fake {
                url: format!("http://127.0.0.1:{port}/mcp"),
                seen,
                _task: task,
            }
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    async fn read_request(stream: &mut TcpStream) -> Option<Seen> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let head = String::from_utf8_lossy(&buf).to_string();
        let mut lines = head.split("\r\n");
        let line = lines.next().unwrap_or("").to_string();
        let mut headers = Vec::new();
        let mut len = 0usize;
        for l in lines {
            if l.is_empty() {
                break;
            }
            if let Some((k, v)) = l.split_once(':') {
                let (k, v) = (k.trim().to_string(), v.trim().to_string());
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.parse().unwrap_or(0);
                }
                headers.push((k, v));
            }
        }
        let head_end = head.find("\r\n\r\n")? + 4;
        let mut body = buf[head_end..].to_vec();
        while body.len() < len {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        let _ = line;
        Some(Seen {
            headers,
            body: String::from_utf8_lossy(&body).to_string(),
        })
    }

    fn tool_answer(id: u64, text: &str, is_error: bool) -> String {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"content": [{"type": "text", "text": text}], "isError": is_error},
        })
        .to_string()
    }

    fn initialize_answer(id: u64, session: &str) -> Answer {
        (
            200,
            Some(session.to_string()),
            json!({"jsonrpc": "2.0", "id": id, "result": {}}).to_string(),
        )
    }

    #[tokio::test]
    async fn handshake_captures_and_resends_the_session() {
        let fake = Fake::spawn(|n, _| match n {
            0 => initialize_answer(1, "sess-1"),
            1 => (200, None, String::new()),
            _ => (200, None, tool_answer(3, r#"{"history":[]}"#, false)),
        })
        .await;
        let mcp = Mcp::new(fake.url.clone(), " sekret ").expect("client");
        let out = mcp
            .call("get_chart_history", json!({"symbol": "EURUSD"}))
            .await
            .expect("a tool answer");
        assert_eq!(out, json!({"history": []}));

        let seen = fake.seen();
        assert_eq!(seen.len(), 3, "initialize, initialized, tools/call");
        let init = seen[0].json();
        assert_eq!(init["method"], "initialize");
        assert_eq!(init["params"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["params"]["clientInfo"]["name"], "chart-bridge");
        assert_eq!(init["params"]["clientInfo"]["version"], "1");
        assert!(
            seen[0].header("Mcp-Session-Id").is_none(),
            "the first request opens the session"
        );
        assert_eq!(seen[1].json()["method"], "notifications/initialized");

        let call = seen[2].json();
        assert_eq!(call["method"], "tools/call");
        assert_eq!(call["params"]["name"], "get_chart_history");
        assert_eq!(seen[2].header("Mcp-Session-Id"), Some("sess-1"));
        assert_eq!(seen[2].header("Authorization"), Some("Bearer sekret"));
        assert_eq!(
            seen[2].header("Accept"),
            Some("application/json, text/event-stream")
        );
        assert_eq!(seen[2].header("Content-Type"), Some("application/json"));
    }

    /// Every reopen scenario in one place: the dropped call is request 2, so a correct client
    /// initializes exactly twice and succeeds on the second call.
    async fn assert_reopens_once(dropped: Answer) {
        let fake = Fake::spawn(move |n, _| match n {
            0 => initialize_answer(1, "sess-1"),
            1 => (200, None, String::new()),
            2 => dropped.clone(),
            3 => initialize_answer(2, "sess-2"),
            4 => (200, None, String::new()),
            _ => (200, None, tool_answer(5, r#"{"history":[1]}"#, false)),
        })
        .await;
        let mcp = Mcp::new(fake.url.clone(), "t").expect("client");
        let out = mcp.call("any_tool", json!({})).await.expect("the retry");
        assert_eq!(out, json!({"history": [1]}));
        let seen = fake.seen();
        let inits = seen
            .iter()
            .filter(|s| s.json()["method"] == "initialize")
            .count();
        assert_eq!(inits, 2, "one reopen, exactly one more initialize");
        assert_eq!(seen.len(), 6);
        assert_eq!(seen[5].header("Mcp-Session-Id"), Some("sess-2"));
    }

    #[tokio::test]
    async fn reopens_once_after_an_http_error() {
        assert_reopens_once((500, None, "boom".to_string())).await;
    }

    #[tokio::test]
    async fn reopens_once_after_a_dropped_connection() {
        assert_reopens_once((0, None, String::new())).await;
    }

    #[tokio::test]
    async fn reopens_once_after_an_unparseable_body() {
        assert_reopens_once((200, None, "<html>nope</html>".to_string())).await;
    }

    #[tokio::test]
    async fn sse_and_plain_bodies_parse_the_same() {
        let text: &str = r#"{"history":[{"time":"2026-11-26T09:00:00"}]}"#;
        let plain = Fake::spawn(move |_, seen| match seen.json()["method"].as_str() {
            Some("tools/call") => (200, None, tool_answer(3, text, false)),
            Some("initialize") => initialize_answer(1, "s"),
            _ => (200, None, String::new()),
        })
        .await;
        let sse = Fake::spawn(move |_, seen| match seen.json()["method"].as_str() {
            Some("tools/call") => {
                let env: Value = serde_json::from_str(&tool_answer(3, text, false)).unwrap();
                (200, None, format!("event: message\ndata: {env}\n\n"))
            }
            Some("initialize") => initialize_answer(1, "s"),
            _ => (200, None, String::new()),
        })
        .await;
        let a = Mcp::new(plain.url.clone(), "t")
            .expect("client")
            .call("x", json!({}))
            .await
            .expect("a plain answer");
        let b = Mcp::new(sse.url.clone(), "t")
            .expect("client")
            .call("x", json!({}))
            .await
            .expect("an SSE answer");
        assert_eq!(a, b);
        assert_eq!(a["history"][0]["time"], "2026-11-26T09:00:00");
    }

    #[tokio::test]
    async fn the_last_data_line_wins() {
        let fake = Fake::spawn(|_, seen| match seen.json()["method"].as_str() {
            Some("tools/call") => {
                let first = tool_answer(3, r#"{"history":[1]}"#, false);
                let last = tool_answer(3, r#"{"history":[2]}"#, false);
                (200, None, format!("data: {first}\ndata: {last}\n\n"))
            }
            Some("initialize") => initialize_answer(1, "s"),
            _ => (200, None, String::new()),
        })
        .await;
        let out = Mcp::new(fake.url.clone(), "t")
            .expect("client")
            .call("x", json!({}))
            .await
            .expect("an answer");
        assert_eq!(out, json!({"history": [2]}));
    }

    #[tokio::test]
    async fn is_error_surfaces_the_text_verbatim() {
        let fake = Fake::spawn(|_, seen| match seen.json()["method"].as_str() {
            Some("tools/call") => (
                200,
                None,
                tool_answer(3, "symbol ERRX does not exist", true),
            ),
            Some("initialize") => initialize_answer(1, "s"),
            _ => (200, None, String::new()),
        })
        .await;
        let err = Mcp::new(fake.url.clone(), "t")
            .expect("client")
            .call("x", json!({}))
            .await
            .expect_err("an isError answer");
        assert_eq!(err.to_string(), "symbol ERRX does not exist");
        assert_eq!(
            fake.seen().len(),
            3,
            "an isError does not reopen the session"
        );
    }

    #[tokio::test]
    async fn a_missing_token_is_a_config_error() {
        let fake = Fake::spawn(|_, _| initialize_answer(1, "s")).await;
        let err = Mcp::new(fake.url.clone(), "   ")
            .expect("client")
            .call("x", json!({}))
            .await
            .expect_err("no token");
        assert_eq!(err.to_string(), "MT5_MCP_TOKEN is not set");
        assert!(fake.seen().is_empty(), "nothing reached the terminal");
    }
}
