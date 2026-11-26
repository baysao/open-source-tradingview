//! The HTTP seam.
//!
//! Two calls are all the app makes: POST to pair, POST to revoke, plus GET /health on the local
//! bridge and on the public edge. Both go through [`Http`] so no test opens a socket.
//!
//! Timeouts are caller-supplied and every call has one. `reqwest` is built with rustls only
//! (D-005's spirit: no OpenSSL on the Windows host).

use std::time::Duration;

/// The pair call's total budget. Generous: the hub contacts the bridge, which contacts MT5.
pub const PAIR_TIMEOUT: Duration = Duration::from_secs(30);
/// Both /health calls answer from memory on the bridge side.
pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(4);
pub const REVOKE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// DNS, connect, TLS or a truncated body — the transport's own words.
    Transport(String),
    /// The deadline passed. Separate because the UI's next action differs: wait and retry, not fix
    /// a URL.
    Timeout,
}

impl HttpError {
    pub fn cause(&self) -> String {
        match self {
            HttpError::Transport(t) => t.clone(),
            HttpError::Timeout => "the request passed its deadline".to_string(),
        }
    }
}

pub trait Http {
    fn get(
        &self,
        url: &str,
        bearer: Option<&str>,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError>;
    fn post_json(
        &self,
        url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError>;

    /// A POST that carries the bearer header. Needed by the MCP token check: the terminal's MCP
    /// answers a tokenless call with 401, so proving the TOKEN works requires sending it. Kept as its
    /// own method rather than an `Option` on `post_json` so the pair/revoke calls cannot silently
    /// start attaching a header they never needed.
    fn post_json_auth(
        &self,
        url: &str,
        body: &str,
        bearer: &str,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError>;
}

/// The production client: rustls, no default TLS, one client reused.
pub struct Real {
    client: reqwest::blocking::Client,
}

impl Real {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .use_rustls_tls()
            // Redirects are refused, not followed: /health never legitimately redirects, and a
            // followed cross-host redirect would carry the bearer header to the redirect target.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("could not build the HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

impl Http for Real {
    fn get(
        &self,
        url: &str,
        bearer: Option<&str>,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        let mut req = self.client.get(url).timeout(timeout);
        if let Some(b) = bearer {
            req = req.header(reqwest::header::AUTHORIZATION, b);
        }
        send(req)
    }

    fn post_json(
        &self,
        url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        let req = self
            .client
            .post(url)
            .timeout(timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_string());
        send(req)
    }

    fn post_json_auth(
        &self,
        url: &str,
        body: &str,
        bearer: &str,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        let req = self
            .client
            .post(url)
            .timeout(timeout)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::AUTHORIZATION, bearer)
            .body(body.to_string());
        send(req)
    }
}

fn send(req: reqwest::blocking::RequestBuilder) -> Result<HttpResponse, HttpError> {
    match req.send() {
        Ok(resp) => {
            let status = resp.status().as_u16();
            // The body is read to the end: a resolved header with an unfinished stream is not an
            // answer (b060: the boot fetch that hung exactly this way).
            let body = resp
                .text()
                .map_err(|e| HttpError::Transport(e.to_string()))?;
            Ok(HttpResponse { status, body })
        }
        Err(e) => {
            if e.is_timeout() {
                Err(HttpError::Timeout)
            } else {
                Err(HttpError::Transport(e.to_string()))
            }
        }
    }
}

/// A scripted responder: one queue per method, plus the request log the tests assert against.
#[derive(Default)]
pub struct Fake {
    pub gets: std::sync::Mutex<Vec<(String, Option<String>)>>,
    pub posts: std::sync::Mutex<Vec<(String, String)>>,
    /// `(url, bearer)` for every authenticated POST, so a test can prove the token travelled.
    pub auths: std::sync::Mutex<Vec<(String, String)>>,
    get_script: std::sync::Mutex<std::collections::VecDeque<Result<HttpResponse, HttpError>>>,
    post_script: std::sync::Mutex<std::collections::VecDeque<Result<HttpResponse, HttpError>>>,
}

impl Fake {
    pub fn with_gets(script: Vec<Result<HttpResponse, HttpError>>) -> Self {
        let me = Self::default();
        *me.get_script.lock().expect("fake http") = script.into();
        me
    }

    pub fn with_posts(script: Vec<Result<HttpResponse, HttpError>>) -> Self {
        let me = Self::default();
        *me.post_script.lock().expect("fake http") = script.into();
        me
    }

    /// Both scripts at once, for a flow that does a GET and a POST (`/health` polls, then the MCP
    /// probe). Two separate constructors would silently leave one of them scripted empty.
    pub fn with_gets_and_posts(
        gets: Vec<Result<HttpResponse, HttpError>>,
        posts: Vec<Result<HttpResponse, HttpError>>,
    ) -> Self {
        let me = Self::default();
        *me.get_script.lock().expect("fake http") = gets.into();
        *me.post_script.lock().expect("fake http") = posts.into();
        me
    }

    pub fn get_calls(&self) -> Vec<(String, Option<String>)> {
        self.gets.lock().expect("fake http").clone()
    }

    pub fn post_calls(&self) -> Vec<(String, String)> {
        self.posts.lock().expect("fake http").clone()
    }

    pub fn auth_calls(&self) -> Vec<(String, String)> {
        self.auths.lock().expect("fake http").clone()
    }

    pub fn total_calls(&self) -> usize {
        self.get_calls().len() + self.post_calls().len()
    }
}

impl Http for Fake {
    fn get(
        &self,
        url: &str,
        bearer: Option<&str>,
        _t: Duration,
    ) -> Result<HttpResponse, HttpError> {
        self.gets
            .lock()
            .expect("fake http")
            .push((url.to_string(), bearer.map(str::to_string)));
        self.get_script
            .lock()
            .expect("fake http")
            .pop_front()
            .unwrap_or_else(|| Err(HttpError::Transport("fake: no scripted GET".to_string())))
    }

    fn post_json(&self, url: &str, body: &str, _t: Duration) -> Result<HttpResponse, HttpError> {
        self.posts
            .lock()
            .expect("fake http")
            .push((url.to_string(), body.to_string()));
        self.post_script
            .lock()
            .expect("fake http")
            .pop_front()
            .unwrap_or_else(|| Err(HttpError::Transport("fake: no scripted POST".to_string())))
    }

    fn post_json_auth(
        &self,
        url: &str,
        body: &str,
        bearer: &str,
        t: Duration,
    ) -> Result<HttpResponse, HttpError> {
        // The bearer is recorded as part of the call, so a test can prove the token was sent and not
        // only that a request happened.
        self.auths
            .lock()
            .expect("fake http")
            .push((url.to_string(), bearer.to_string()));
        self.post_json(url, body, t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fake_records_method_url_and_bearer() {
        let fake = Fake::with_gets(vec![Ok(HttpResponse {
            status: 200,
            body: "{}".into(),
        })]);
        fake.get(
            "http://127.0.0.1:8797/health",
            Some("Bearer t"),
            HEALTH_TIMEOUT,
        )
        .expect("scripted");
        assert_eq!(
            fake.get_calls(),
            vec![(
                "http://127.0.0.1:8797/health".to_string(),
                Some("Bearer t".to_string())
            )]
        );
    }

    #[test]
    fn an_unscripted_post_is_an_error_not_a_pass() {
        let fake = Fake::default();
        let err = fake
            .post_json("https://h/api", "{}", PAIR_TIMEOUT)
            .expect_err("unscripted");
        assert!(err.cause().contains("no scripted POST"));
    }

    #[test]
    fn a_timeout_keeps_its_own_cause() {
        assert_eq!(
            HttpError::Timeout.cause(),
            "the request passed its deadline"
        );
    }
}
