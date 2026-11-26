//! The outbound caller for user bridges (07-ARCHITECTURE): vetted URL, pinned connect, no
//! redirects, the spec's limits, and the seven exact error causes CUJ 7 shows. Tests construct
//! [`Vetted`] by hand over a plain-HTTP loopback fake (the https-ness is the guard's contract,
//! proven in [`super::guard`]); the client dials what it is handed.
//!
//! Limits (spec `:360`): 10 s per call, 5 MB response cap, no redirects, at most 2 concurrent
//! calls per bridge. The `/wait` long-poll runs up to 20 s on the bridge, so its budget is 25 s
//! (architecture decision 4). Tests pass smaller budgets instead of sleeping for seconds.

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt;
use tokio::sync::Semaphore;
use uuid::Uuid;

use super::guard::{Refusal, Vetted};

/// The spec's call limits (spec `:360`). Tests build smaller budgets instead of sleeping.
#[derive(Clone, Debug)]
pub struct BridgeLimits {
    /// Every call but `wait`.
    pub call_timeout: Duration,
    /// The `/wait` long-poll: the bridge long-polls 20 s, so 25 s (architecture decision 4).
    pub wait_timeout: Duration,
    /// The response cap (5 MB in production).
    pub max_bytes: usize,
}

impl Default for BridgeLimits {
    fn default() -> Self {
        Self {
            call_timeout: Duration::from_secs(10),
            wait_timeout: Duration::from_secs(25),
            max_bytes: 5 * 1024 * 1024,
        }
    }
}

/// Every way a bridge call can fail, one variant per CUJ 7 cause. The `Display` text reaches the
/// Settings screen verbatim.
#[derive(Debug)]
pub enum BridgeError {
    /// The SSRF guard refused the address; the string is the guard's own reason.
    Refused(&'static str),
    /// The host did not resolve.
    Dns,
    /// The TLS handshake or certificate failed.
    Cert,
    /// The bridge answered 401 or 403: the token is wrong.
    Auth,
    /// The call exceeded its total budget.
    Timeout,
    /// The response body passed the size cap.
    TooLarge,
    /// The bridge answered with an error, the connection failed, or the body was not JSON.
    Bridge(String),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::Refused(r) => write!(f, "the bridge address was refused: {r}"),
            BridgeError::Dns => write!(f, "the bridge host did not resolve"),
            BridgeError::Cert => write!(f, "certificate error"),
            BridgeError::Auth => write!(f, "the bridge refused the token"),
            BridgeError::Timeout => write!(f, "the bridge did not answer in time"),
            BridgeError::TooLarge => write!(f, "the response exceeded the size cap"),
            BridgeError::Bridge(s) => write!(f, "{s}"),
        }
    }
}

impl From<Refusal> for BridgeError {
    fn from(r: Refusal) -> Self {
        match r {
            Refusal::Host(reason) => BridgeError::Refused(reason),
            other => BridgeError::Bridge(other.to_string()),
        }
    }
}

/// One HTTP client per `(host, port, pinned addrs)`: the pin is a builder setting, so a re-vet
/// with different addresses builds a new entry; stale entries are small and unreachable, never
/// wrong.
type PinnedClients = HashMap<(String, u16, Vec<IpAddr>), reqwest::Client>;

/// The outbound caller: the pinned-client cache above, and one semaphore of 2 permits per
/// bridge. Shared through an `Arc` in [`super::Bridges`]: the pinned clients and semaphores
/// must stay one set per process, so a clone of the state cannot fork them.
pub struct BridgeClient {
    limits: BridgeLimits,
    http: Mutex<PinnedClients>,
    sems: Mutex<HashMap<Uuid, Arc<Semaphore>>>,
}

impl Default for BridgeClient {
    fn default() -> Self {
        Self::new()
    }
}

impl BridgeClient {
    /// The production limits.
    pub fn new() -> Self {
        Self::with_limits(BridgeLimits::default())
    }

    /// Tests: smaller timeouts and caps instead of sleeping.
    pub fn with_limits(limits: BridgeLimits) -> Self {
        Self {
            limits,
            http: Mutex::new(HashMap::new()),
            sems: Mutex::new(HashMap::new()),
        }
    }

    /// The HTTP client for this vetted endpoint: redirects off, connect pinned to the vetted
    /// addresses. `resolve_to_addrs` is a builder setting, hence the cache keyed by the pin.
    fn http_for(&self, v: &Vetted) -> Result<reqwest::Client, BridgeError> {
        let key = (v.host.clone(), v.port, v.addrs.clone());
        if let Some(c) = self.http.lock().unwrap().get(&key) {
            return Ok(c.clone());
        }
        let addrs: Vec<SocketAddr> = v
            .addrs
            .iter()
            .map(|ip| SocketAddr::new(*ip, v.port))
            .collect();
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        builder = builder.resolve_to_addrs(&key.0, addrs.as_slice());
        let client = builder
            .build()
            .map_err(|e| BridgeError::Bridge(format!("http client: {e}")))?;
        self.http.lock().unwrap().insert(key, client.clone());
        Ok(client)
    }

    /// Reads the body under the cap: an early content-length check, then a streamed accumulate.
    async fn read_limited(&self, resp: reqwest::Response) -> Result<Vec<u8>, BridgeError> {
        if let Some(len) = resp.content_length() {
            if len as usize > self.limits.max_bytes {
                return Err(BridgeError::TooLarge);
            }
        }
        let mut out = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(map_req_err)?;
            if out.len() + chunk.len() > self.limits.max_bytes {
                return Err(BridgeError::TooLarge);
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// One bridge call: permit, dial, status mapping, cap, JSON. `wait = true` uses the long-poll
    /// budget (25 s) instead of the 10 s default.
    async fn call(
        &self,
        bridge: &Uuid,
        v: &Vetted,
        token: &str,
        path_query: &str,
        wait: bool,
    ) -> Result<serde_json::Value, BridgeError> {
        let sem = self
            .sems
            .lock()
            .unwrap()
            .entry(*bridge)
            .or_insert_with(|| Arc::new(Semaphore::new(2)))
            .clone();
        let _permit = sem
            .acquire()
            .await
            .map_err(|_| BridgeError::Bridge("bridge semaphore closed".into()))?;
        let client = self.http_for(v)?;
        let base = v.url.as_str().trim_end_matches('/');
        let url = reqwest::Url::parse(&format!("{base}/{path_query}"))
            .map_err(|_| BridgeError::Bridge("bad endpoint url".into()))?;
        let timeout = if wait {
            self.limits.wait_timeout
        } else {
            self.limits.call_timeout
        };
        let resp = client
            .get(url)
            .bearer_auth(token)
            .timeout(timeout)
            .send()
            .await
            .map_err(map_req_err)?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(BridgeError::Auth);
        }
        if status.is_redirection() {
            return Err(BridgeError::Bridge(
                "redirect not followed (the client never follows redirects)".into(),
            ));
        }
        let body = self.read_limited(resp).await?;
        if !status.is_success() {
            return Err(BridgeError::Bridge(bridge_message(status, &body)));
        }
        serde_json::from_slice(&body).map_err(|e| BridgeError::Bridge(format!("bad json: {e}")))
    }

    /// `GET /health`: the terminal's name and state, as CUJ 7's Test shows.
    pub async fn health(
        &self,
        bridge: &Uuid,
        v: &Vetted,
        token: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.call(bridge, v, token, "health", false).await
    }

    /// `GET /symbols`: every tradable symbol of the terminal.
    pub async fn symbols(
        &self,
        bridge: &Uuid,
        v: &Vetted,
        token: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        self.call(bridge, v, token, "symbols", false).await
    }

    /// `GET /bars?symbol=&tf=&limit=&to=`: candles, newest last.
    pub async fn bars(
        &self,
        bridge: &Uuid,
        v: &Vetted,
        token: &str,
        q: BarsQuery<'_>,
    ) -> Result<serde_json::Value, BridgeError> {
        let mut pairs = vec![
            ("symbol", q.symbol.to_string()),
            ("tf", q.tf.to_string()),
            ("limit", q.limit.to_string()),
        ];
        if let Some(to) = q.to {
            pairs.push(("to", to.to_string()));
        }
        let path = format!("bars?{}", q_str(&pairs));
        self.call(bridge, v, token, &path, false).await
    }

    /// `GET /wait?symbol=&tf=&after=`: the long-poll. 25 s budget (the bridge long-polls 20 s);
    /// spends one of the bridge's two concurrency permits.
    pub async fn wait(
        &self,
        bridge: &Uuid,
        v: &Vetted,
        token: &str,
        symbol: &str,
        tf: &str,
        after: &str,
    ) -> Result<serde_json::Value, BridgeError> {
        let pairs = vec![
            ("symbol", symbol.to_string()),
            ("tf", tf.to_string()),
            ("after", after.to_string()),
        ];
        self.call(bridge, v, token, &format!("wait?{}", q_str(&pairs)), true)
            .await
    }
}

/// The `/bars` query: symbol, timeframe, bounded limit, optional `to` (newest bar, UTC ms).
#[derive(Debug, Clone, Copy)]
pub struct BarsQuery<'a> {
    pub symbol: &'a str,
    pub tf: &'a str,
    pub limit: u32,
    pub to: Option<i64>,
}

/// Maps a transport error onto the exact CUJ 7 cause: timeout, TLS/certificate, everything else
/// keeps the reqwest text (connection refused, dns, and so on).
fn map_req_err(e: reqwest::Error) -> BridgeError {
    if e.is_timeout() {
        return BridgeError::Timeout;
    }
    let text = e.to_string();
    let lower = text.to_ascii_lowercase();
    if lower.contains("certificate") || lower.contains("tls") || lower.contains("rustls") {
        return BridgeError::Cert;
    }
    BridgeError::Bridge(text)
}

/// The bridge's own error text for a non-2xx status: its JSON `error` field when it sent one,
/// the canonical reason phrase when it did not.
fn bridge_message(status: reqwest::StatusCode, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            return err.to_string();
        }
    }
    format!("the bridge answered {}: {}", status.as_u16(), text.trim())
}

/// A percent-encoded query string; the bridge parses it with `parse_qs`, which decodes.
fn q_str(pairs: &[(&str, String)]) -> String {
    use percent_encoding::{percent_encode, NON_ALPHANUMERIC};
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v.as_bytes(), NON_ALPHANUMERIC)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    type Hits = Arc<std::sync::Mutex<Vec<String>>>;

    /// A minimal HTTP/1.1 fake bridge: one connection per request (`Connection: close`), a fixed
    /// route table, a per-request delay, and a `/big` body of `big_bytes`. `/redirect` points at
    /// a bound-then-dropped port, so following it would fail to connect; the client must refuse
    /// the redirect before that can happen.
    struct Fake {
        port: u16,
        hits: Hits,
        /// The highest number of simultaneously open connections (the semaphore's proof).
        max_conc: Arc<AtomicUsize>,
        _keep: tokio::task::JoinHandle<()>,
    }

    fn reason(status: u16) -> &'static str {
        match status {
            200 => "OK",
            302 => "Found",
            401 => "Unauthorized",
            500 => "Internal Server Error",
            _ => "OK",
        }
    }

    impl Fake {
        async fn spawn(delay_ms: u64, big_bytes: usize) -> Fake {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            let port = listener.local_addr().unwrap().port();
            let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind closed target");
            let closed_port = closed.local_addr().unwrap().port();
            drop(closed);
            let hits: Hits = Arc::new(std::sync::Mutex::new(Vec::new()));
            let max_conc = Arc::new(AtomicUsize::new(0));
            let cur = Arc::new(AtomicUsize::new(0));
            let (h, m, c) = (hits.clone(), max_conc.clone(), cur.clone());
            let keep = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = match listener.accept().await {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    let (h, m, c) = (h.clone(), m.clone(), c.clone());
                    tokio::spawn(async move {
                        let cur_n = c.fetch_add(1, Ordering::SeqCst) + 1;
                        m.fetch_max(cur_n, Ordering::SeqCst);
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 1024];
                        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
                            match stream.read(&mut chunk).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                            }
                        }
                        let text = String::from_utf8_lossy(&buf).to_string();
                        let path = text.split_whitespace().nth(1).unwrap_or("").to_string();
                        h.lock().unwrap().push(path.clone());
                        if delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        }
                        let (status, headers, body): (u16, Vec<(&str, String)>, Vec<u8>) =
                            match path.split('?').next().unwrap_or("") {
                                "/health" => (200, vec![], br#"{"ok":true,"server":"fake"}"#.to_vec()),
                                "/symbols" => (
                                    200,
                                    vec![],
                                    br#"[{"name":"EURUSD","desc":"","digits":5,"path":"Forex"}]"#
                                        .to_vec(),
                                ),
                                "/bars" => (
                                    200,
                                    vec![],
                                    br#"[{"time":1,"open":1.0,"high":1.0,"low":1.0,"close":1.0,"volume":0.0}]"#
                                        .to_vec(),
                                ),
                                "/locked" => (401, vec![], br#"{"error":"unauthorized"}"#.to_vec()),
                                "/wait" => (
                                    200,
                                    vec![],
                                    br#"{"bar":null,"key":"","tick_ms":null}"#.to_vec(),
                                ),
                                "/junk" => (200, vec![], b"not json".to_vec()),
                                "/big" => (200, vec![], vec![b'x'; big_bytes]),
                                "/redirect" => (
                                    302,
                                    vec![(
                                        "Location",
                                        format!("http://127.0.0.1:{closed_port}/x"),
                                    )],
                                    vec![],
                                ),
                                _ => (404, vec![], br#"{"error":"not found"}"#.to_vec()),
                            };
                        let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
                        head.push_str("Content-Type: application/json\r\n");
                        for (k, v) in &headers {
                            head.push_str(&format!("{k}: {v}\r\n"));
                        }
                        head.push_str(&format!(
                            "Content-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        ));
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream.write_all(&body).await;
                        let _ = stream.flush().await;
                        c.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            });
            Fake {
                port,
                hits,
                max_conc,
                _keep: keep,
            }
        }

        fn vetted(&self) -> Vetted {
            let url = reqwest::Url::parse(&format!("http://127.0.0.1:{}", self.port)).unwrap();
            Vetted {
                url,
                host: "127.0.0.1".into(),
                port: self.port,
                addrs: vec![IpAddr::from([127, 0, 0, 1])],
            }
        }
    }

    // tokio::io traits for the fake's reader/writer.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn limits(call_ms: u64, wait_ms: u64, max_bytes: usize) -> BridgeLimits {
        BridgeLimits {
            call_timeout: Duration::from_millis(call_ms),
            wait_timeout: Duration::from_millis(wait_ms),
            max_bytes,
        }
    }

    #[tokio::test]
    async fn health_symbols_bars_roundtrip() {
        let fake = Fake::spawn(0, 0).await;
        let client = BridgeClient::new();
        let v = fake.vetted();
        let id = Uuid::now_v7();
        let h = client.health(&id, &v, "tok").await.expect("health");
        assert_eq!(h["ok"], serde_json::json!(true));
        let s = client.symbols(&id, &v, "tok").await.expect("symbols");
        assert_eq!(s.as_array().unwrap().len(), 1);
        let b = client
            .bars(
                &id,
                &v,
                "tok",
                BarsQuery {
                    symbol: "EURUSD",
                    tf: "H1",
                    limit: 100,
                    to: Some(1730000000000),
                },
            )
            .await
            .expect("bars");
        assert_eq!(b.as_array().unwrap()[0]["time"], serde_json::json!(1));
        let hits = fake.hits.lock().unwrap().clone();
        assert!(hits.iter().any(|p| p.starts_with("/health")));
        assert!(
            hits.iter().any(|p| p.starts_with("/bars?symbol=EURUSD")),
            "the query reached the bridge: {hits:?}"
        );
    }

    #[tokio::test]
    async fn auth_junk_and_redirect_causes() {
        let fake = Fake::spawn(0, 0).await;
        let client = BridgeClient::new();
        let v = fake.vetted();
        let id = Uuid::now_v7();
        // The fake's /locked answers 401 with the JSON error shape; call() maps it to Auth.
        let err = client
            .call(&id, &v, "tok", "locked", false)
            .await
            .unwrap_err();
        assert!(matches!(err, BridgeError::Auth), "{err}");
        // /junk answers 200 with a non-JSON body.
        let err = client
            .call(&id, &v, "tok", "junk", false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BridgeError::Bridge(ref s) if s.contains("bad json")),
            "{err}"
        );
        // /redirect answers 302; the client refuses it before connecting anywhere.
        let err = client
            .call(&id, &v, "tok", "redirect", false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, BridgeError::Bridge(ref s) if s.contains("redirect not followed")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn too_large_and_timeout() {
        // /big answers immediately; the cap (64) trips on the content-length (128).
        let big = Fake::spawn(0, 128).await;
        let client = BridgeClient::with_limits(limits(500, 500, 64));
        let v = big.vetted();
        let id = Uuid::now_v7();
        let err = client.call(&id, &v, "tok", "big", false).await.unwrap_err();
        assert!(matches!(err, BridgeError::TooLarge), "{err}");
        // A slow bridge against a short budget: Timeout.
        let slow = Fake::spawn(400, 0).await;
        let client = BridgeClient::with_limits(limits(100, 100, 64));
        let v = slow.vetted();
        let err = client
            .call(&id, &v, "tok", "health", false)
            .await
            .unwrap_err();
        assert!(matches!(err, BridgeError::Timeout), "{err}");
    }

    #[tokio::test]
    async fn wait_uses_the_long_budget() {
        let fake = Fake::spawn(60, 0).await;
        // call_timeout 20 ms would time out; wait's 500 ms budget must carry it.
        let client = BridgeClient::with_limits(limits(20, 500, 4096));
        let v = fake.vetted();
        let id = Uuid::now_v7();
        let r = client
            .wait(&id, &v, "tok", "EURUSD", "M1", "")
            .await
            .expect("wait within the long budget");
        assert!(r.get("bar").is_none() || r["bar"].is_null(), "{r}");
    }

    #[tokio::test]
    async fn two_permits_third_waits() {
        let fake = Fake::spawn(150, 0).await;
        let client = BridgeClient::new();
        let v = fake.vetted();
        let id = Uuid::now_v7();
        // Three concurrent calls through one client: the semaphore of 2 caps the fake's
        // concurrency, and all three still succeed.
        let (a, b, c) = tokio::join!(
            client.call(&id, &v, "tok", "health", false),
            client.call(&id, &v, "tok", "health", false),
            client.call(&id, &v, "tok", "health", false),
        );
        a.expect("a");
        b.expect("b");
        c.expect("c");
        assert_eq!(
            fake.max_conc.load(Ordering::SeqCst),
            2,
            "the semaphore of 2 held"
        );
    }
}
