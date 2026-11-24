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

/// A per-client-IP token bucket for `/api/*`, `/auth/*` and `/admin/*`: `rate` tokens per
/// second, `burst` at most.
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

/// The paths the limiter guards. Hosted auth endpoints are as exposed as the API, and the
/// admin log routes read files, so all three areas share one bucket.
pub fn is_limited(path: &str) -> bool {
    ["/api/", "/auth/", "/admin/"]
        .iter()
        .any(|p| path.starts_with(p))
}

/// Middleware: 429 with `Retry-After` when a client IP empties its bucket on a guarded path.
pub async fn rate_limit(
    axum::extract::State(limiter): axum::extract::State<RateLimiter>,
    req: Request<Body>,
    next: Next,
) -> Response {
    if is_limited(req.uri().path()) {
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
        if let Err(wait) = limiter.take(ip, Instant::now()) {
            let mut resp = api_error(StatusCode::TOO_MANY_REQUESTS, "too many requests");
            resp.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(wait));
            return resp;
        }
    }
    next.run(req).await
}

/// Middleware: `X-Content-Type-Options: nosniff` and `Referrer-Policy: same-origin` on every
/// response. The CSP is set in Phase 4, because the current pages use inline scripts.
pub async fn security_headers(req: Request<Body>, next: Next) -> Response {
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
    resp
}

#[cfg(test)]
mod guard_tests {
    use super::*;

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
    fn limiter_covers_api_auth_and_admin() {
        for p in ["/api/bars", "/auth/start", "/auth/me", "/admin/logs"] {
            assert!(is_limited(p), "{p} is limited");
        }
        for p in ["/", "/hub.html", "/ws", "/logout"] {
            assert!(!is_limited(p), "{p} is not limited");
        }
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
