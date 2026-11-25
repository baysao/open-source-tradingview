//! Client identity at the edge. 01-02 owns `ClientIp`; 01-03 adds the rate limiter, headers and
//! the POST guard.

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

/// The client address: `CF-Connecting-IP` when the TCP peer is loopback (the Cloudflare tunnel
/// connects from 127.0.0.1), else the TCP peer. `peer_loopback` tells whether the TCP peer itself was
/// loopback (the local-mode socket exemption keys on it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientIp {
    pub ip: IpAddr,
    pub peer_loopback: bool,
}

/// Pure: the rule above for a peer and an optional header value.
pub fn client_ip(peer: IpAddr, cf_connecting_ip: Option<&str>) -> ClientIp {
    let peer_loopback = peer.is_loopback();
    let ip = if peer_loopback {
        cf_connecting_ip
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
            .unwrap_or(peer)
    } else {
        peer
    };
    ClientIp { ip, peer_loopback }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        let header = parts
            .headers
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok());
        Ok(client_ip(peer, header))
    }
}

// ---------------------------------------------------------------- 01-03: guard, limits, headers

use crate::config::Config;
use crate::routes::api_error;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::Instant;

/// Python's POST guard (`hub/server.py:989-1000`): an `Origin`, when present, must be this server
/// (`http://localhost:<port>`, `http://127.0.0.1:<port>`, or `PUBLIC_ORIGIN`), else 403; the body
/// must be JSON, else 415; over 300,000 bytes is 413. A missing `Origin` is allowed (curl, agents).
pub fn post_guard(cfg: &Config, headers: &HeaderMap, body_len: usize) -> Result<(), Box<Response>> {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let allowed = [
            format!("http://localhost:{}", cfg.http_port),
            format!("http://127.0.0.1:{}", cfg.http_port),
            cfg.public_origin.clone(),
        ];
        if !allowed.iter().any(|a| a == origin) {
            return Err(Box::new(api_error(
                StatusCode::FORBIDDEN,
                "forbidden origin",
            )));
        }
    }
    let ctype = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !ctype.contains("application/json") {
        return Err(Box::new(api_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "JSON only",
        )));
    }
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if declared.max(body_len) > 300_000 {
        return Err(Box::new(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too large",
        )));
    }
    Ok(())
}

/// A per-client-IP token bucket: `rate` tokens per second, `burst` at most.
#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, (f64, Instant)>>>,
    rate: f64,
    burst: f64,
}

impl RateLimiter {
    /// The 01-03 default is 20 requests per second, burst 40.
    pub fn new(rate: f64, burst: f64) -> RateLimiter {
        RateLimiter {
            inner: Arc::new(Mutex::new(HashMap::new())),
            rate,
            burst,
        }
    }

    /// Takes one token for `ip` at `now`; `Err(seconds)` to wait when empty.
    pub fn take(&self, ip: IpAddr, now: Instant) -> Result<(), u64> {
        let mut m = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if m.len() > 10_000 {
            m.retain(|_, (_, t)| now.duration_since(*t).as_secs() < 60);
        }
        let (tokens, last) = m.entry(ip).or_insert((self.burst, now));
        *tokens = (*tokens + now.duration_since(*last).as_secs_f64() * self.rate).min(self.burst);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            Ok(())
        } else {
            Err(((1.0 - *tokens) / self.rate).ceil().max(1.0) as u64)
        }
    }
}

/// The four areas the limiter scopes to. Each has its own bucket, so exhausting one never slows
/// another: `/ws` upgrades are cheap to open and expensive to hold, and `/auth/*` is the one an
/// attacker would hammer for credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    Api,
    Auth,
    Admin,
    Ws,
}

/// `/api/*`, `/auth/*`, `/admin/*` by prefix and `/ws` exactly (never `/websocket`). `/api/metrics`
/// is an `Api` call: the beacon is flood-controlled by the same bucket as the API it reports on.
pub fn bucket(path: &str) -> Option<Bucket> {
    if path.starts_with("/api/") {
        return Some(Bucket::Api);
    }
    if path.starts_with("/auth/") {
        return Some(Bucket::Auth);
    }
    if path.starts_with("/admin/") {
        return Some(Bucket::Admin);
    }
    if path == "/ws" {
        return Some(Bucket::Ws);
    }
    None
}

/// The four buckets, sized per area. The values live here and nowhere else: no environment
/// variable moves them (an operator-tunable limiter is a new invalid-input path for no need).
#[derive(Clone)]
pub struct RateLimits {
    hosted: bool,
    api: RateLimiter,
    auth: RateLimiter,
    admin: RateLimiter,
    ws: RateLimiter,
}

/// `/api/*`: one request per 50 ms is a busy chart page (each open chart polls).
const API_PER_SEC: f64 = 20.0;
const API_BURST: f64 = 40.0;
/// `/auth/*` and `/admin/*`: sign-in is a handful of calls; brute force is the threat.
const AUTH_PER_SEC: f64 = 5.0;
const AUTH_BURST: f64 = 10.0;
const ADMIN_PER_SEC: f64 = 5.0;
const ADMIN_BURST: f64 = 10.0;
/// `/ws`: a page opens a few sockets; the per-IP cap (`Limits.per_ip`) bounds the rest.
const WS_PER_SEC: f64 = 10.0;
const WS_BURST: f64 = 30.0;

impl RateLimits {
    /// Hosted mode limits `/ws`; local mode does not (every tab is 127.0.0.1, so a page reload
    /// storm is one IP, and the local contract suite hammers the API).
    pub fn new(hosted: bool) -> RateLimits {
        RateLimits {
            hosted,
            api: RateLimiter::new(API_PER_SEC, API_BURST),
            auth: RateLimiter::new(AUTH_PER_SEC, AUTH_BURST),
            admin: RateLimiter::new(ADMIN_PER_SEC, ADMIN_BURST),
            ws: RateLimiter::new(WS_PER_SEC, WS_BURST),
        }
    }

    /// The bucket guarding `path` in this mode, or `None` when it is not limited.
    pub fn bucket_for(&self, path: &str) -> Option<Bucket> {
        let b = bucket(path)?;
        if b == Bucket::Ws && !self.hosted {
            return None;
        }
        Some(b)
    }

    /// Takes one token from `b` for `ip`; `Err(seconds)` to wait when empty.
    pub fn take(&self, b: Bucket, ip: IpAddr, now: Instant) -> Result<(), u64> {
        let limiter = match b {
            Bucket::Api => &self.api,
            Bucket::Auth => &self.auth,
            Bucket::Admin => &self.admin,
            Bucket::Ws => &self.ws,
        };
        limiter.take(ip, now)
    }
}

/// Middleware: 429 with `Retry-After` when a client IP empties its bucket on a guarded path.
pub async fn rate_limit(
    axum::extract::State(rates): axum::extract::State<RateLimits>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if let Some(b) = rates.bucket_for(req.uri().path()) {
        let peer = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        let header_ip = req
            .headers()
            .get("cf-connecting-ip")
            .and_then(|v| v.to_str().ok());
        let ip = client_ip(peer, header_ip).ip;
        if let Err(wait) = rates.take(b, ip, Instant::now()) {
            let mut resp = api_error(StatusCode::TOO_MANY_REQUESTS, "too many requests");
            resp.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(wait));
            return resp;
        }
    }
    next.run(req).await
}

/// The response headers every request gets, and the policy a response carries when the handler set
/// none of its own.
#[derive(Clone)]
pub struct HeaderPolicy {
    pub default_csp: HeaderValue,
}

impl HeaderPolicy {
    pub fn new(default_csp: HeaderValue) -> HeaderPolicy {
        HeaderPolicy { default_csp }
    }
}

/// Middleware: `X-Content-Type-Options: nosniff`, `Referrer-Policy: same-origin` and a CSP on every
/// response. The CSP is inserted only when the handler set none: the static route puts a page's own
/// hashed policy in place, and layers run inside-out on the way back, so that value is visible here.
pub async fn security_headers(
    axum::extract::State(policy): axum::extract::State<HeaderPolicy>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    h.entry(header::CONTENT_SECURITY_POLICY)
        .or_insert(policy.default_csp);
    resp
}

#[cfg(test)]
mod guard_tests {
    use super::*;
    use axum::response::IntoResponse as _;

    fn mk(origin: Option<&str>, ctype: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(o) = origin {
            h.insert(header::ORIGIN, HeaderValue::from_str(o).unwrap());
        }
        h.insert(header::CONTENT_TYPE, HeaderValue::from_str(ctype).unwrap());
        h
    }

    #[test]
    fn post_guard_matches_python() {
        let cfg = Config::from_lookup(|_| None).unwrap();
        assert!(post_guard(&cfg, &mk(None, "application/json"), 10).is_ok());
        assert!(post_guard(
            &cfg,
            &mk(
                Some("http://localhost:8770"),
                "application/json; charset=utf-8"
            ),
            10
        )
        .is_ok());
        let status = |r: Result<(), Box<Response>>| r.unwrap_err().status();
        assert_eq!(
            status(post_guard(
                &cfg,
                &mk(Some("http://evil.example"), "application/json"),
                10
            )),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status(post_guard(&cfg, &mk(None, "text/plain"), 10)),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
        assert_eq!(
            status(post_guard(&cfg, &mk(None, "application/json"), 300_001)),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[test]
    fn paths_scope_to_their_bucket() {
        for p in ["/api/bars", "/api/metrics/ttfc", "/api/me"] {
            assert_eq!(bucket(p), Some(Bucket::Api), "{p}");
        }
        for p in ["/auth/start", "/auth/me", "/auth/google/onetap"] {
            assert_eq!(bucket(p), Some(Bucket::Auth), "{p}");
        }
        assert_eq!(bucket("/admin/logs"), Some(Bucket::Admin));
        assert_eq!(bucket("/admin/metrics"), Some(Bucket::Admin));
        assert_eq!(bucket("/ws"), Some(Bucket::Ws), "the upgrade is limited");
        // Exactly `/ws`: a sibling path must not inherit the bucket.
        for p in ["/", "/hub.html", "/logout", "/websocket", "/ws/", "/authx"] {
            assert_eq!(bucket(p), None, "{p} is not limited");
        }
    }

    #[test]
    fn the_ws_bucket_is_hosted_only() {
        assert_eq!(RateLimits::new(true).bucket_for("/ws"), Some(Bucket::Ws));
        assert_eq!(RateLimits::new(false).bucket_for("/ws"), None);
        // The other three are the same in both modes: local mode has no auth or admin route, and
        // the contract suite relies on the Api values.
        for hosted in [true, false] {
            let r = RateLimits::new(hosted);
            assert_eq!(r.bucket_for("/api/bars"), Some(Bucket::Api));
            assert_eq!(r.bucket_for("/auth/me"), Some(Bucket::Auth));
            assert_eq!(r.bucket_for("/admin/logs"), Some(Bucket::Admin));
        }
    }

    #[test]
    fn each_bucket_is_independent() {
        let r = RateLimits::new(true);
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        let t0 = Instant::now();
        // Drain Auth (10) and Admin (10) and Ws (30) completely.
        for _ in 0..10 {
            assert!(r.take(Bucket::Auth, ip, t0).is_ok());
            assert!(r.take(Bucket::Admin, ip, t0).is_ok());
        }
        for _ in 0..30 {
            assert!(r.take(Bucket::Ws, ip, t0).is_ok());
        }
        assert!(r.take(Bucket::Auth, ip, t0).is_err(), "11th auth call");
        assert!(r.take(Bucket::Admin, ip, t0).is_err(), "11th admin call");
        assert!(r.take(Bucket::Ws, ip, t0).is_err(), "31st ws upgrade");
        // Api is untouched by three exhausted neighbours.
        for _ in 0..40 {
            assert!(r.take(Bucket::Api, ip, t0).is_ok());
        }
        assert!(r.take(Bucket::Api, ip, t0).is_err(), "41st api call");
        // The rate is per second: the wait is reported, never a silent pass.
        assert_eq!(r.take(Bucket::Auth, ip, t0), Err(1));
    }

    /// The burst proves the cap (`10`); only the refill proves the *rate*, which is the number an
    /// attacker meets after a one-second pause. Nothing below asserts an absolute wait, so the test
    /// is independent of `Retry-After` rounding and moves only when a `*_PER_SEC` value moves.
    #[test]
    fn each_bucket_refills_at_its_own_rate() {
        let r = RateLimits::new(true);
        let ip: IpAddr = "203.0.113.11".parse().unwrap();
        let t0 = Instant::now();
        let t1 = t0 + std::time::Duration::from_secs(1);

        // Auth: burst 10, then 5 more in the next second and not one more.
        for _ in 0..10 {
            assert!(r.take(Bucket::Auth, ip, t0).is_ok());
        }
        assert!(r.take(Bucket::Auth, ip, t0).is_err(), "auth burst spent");
        for i in 0..5 {
            assert!(
                r.take(Bucket::Auth, ip, t1).is_ok(),
                "auth token {i} at 5/s"
            );
        }
        assert!(
            r.take(Bucket::Auth, ip, t1).is_err(),
            "the 6th auth call inside one second waits"
        );

        // Api refills 20 in that same second, so the two rates are separate values, not one shared.
        for _ in 0..40 {
            assert!(r.take(Bucket::Api, ip, t0).is_ok());
        }
        assert!(r.take(Bucket::Api, ip, t0).is_err(), "api burst spent");
        for i in 0..20 {
            assert!(r.take(Bucket::Api, ip, t1).is_ok(), "api token {i} at 20/s");
        }
        assert!(
            r.take(Bucket::Api, ip, t1).is_err(),
            "the 21st api call inside one second waits"
        );
    }

    #[test]
    fn the_headers_policy_is_inserted_only_when_absent() {
        let p = HeaderPolicy::new(HeaderValue::from_static("default-src 'none'"));
        let mut resp = StatusCode::OK.into_response();
        resp.headers_mut()
            .entry(header::CONTENT_SECURITY_POLICY)
            .or_insert(p.default_csp.clone());
        assert_eq!(
            resp.headers().get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "default-src 'none'"
        );
        // A handler-set per-page policy wins.
        let mut page = StatusCode::OK.into_response();
        page.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("script-src 'self'"),
        );
        page.headers_mut()
            .entry(header::CONTENT_SECURITY_POLICY)
            .or_insert(p.default_csp.clone());
        assert_eq!(
            page.headers().get(header::CONTENT_SECURITY_POLICY).unwrap(),
            "script-src 'self'"
        );
    }

    #[test]
    fn token_bucket() {
        let l = RateLimiter::new(20.0, 40.0);
        let ip: IpAddr = "203.0.113.1".parse().unwrap();
        let t0 = Instant::now();
        for _ in 0..40 {
            assert!(l.take(ip, t0).is_ok());
        }
        assert!(l.take(ip, t0).is_err(), "burst spent");
        assert!(
            l.take(ip, t0 + std::time::Duration::from_millis(60))
                .is_ok(),
            "refills at 20/s"
        );
        assert!(l.take("203.0.113.2".parse().unwrap(), t0).is_ok(), "per IP");
    }
}

#[cfg(test)]
mod tests {
    use super::client_ip;
    use std::net::IpAddr;

    #[test]
    fn header_trusted_only_from_loopback() {
        let lo: IpAddr = "127.0.0.1".parse().unwrap();
        let remote: IpAddr = "198.51.100.9".parse().unwrap();
        assert_eq!(
            client_ip(lo, Some("203.0.113.5")).ip,
            "203.0.113.5".parse::<IpAddr>().unwrap()
        );
        assert_eq!(client_ip(remote, Some("203.0.113.5")).ip, remote);
        assert_eq!(client_ip(lo, Some("garbage")).ip, lo);
        assert!(client_ip(lo, None).peer_loopback);
        assert!(!client_ip(remote, None).peer_loopback);
    }
}
