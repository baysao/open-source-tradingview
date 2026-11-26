//! `status`: three answers, each with the question it answers.
//!
//! 1. **local bridge /health** — "is the bridge process answering, and how long ago?" Uses the
//!    bearer header and a bounded deadline; a slow answer is a timeout, not a hung panel.
//! 2. **task states** — "are the two scheduled tasks registered, and running?"
//! 3. **public edge /health** — "does the URL the hub registers actually answer?" That is the
//!    tunnel URL, not 127.0.0.1: a bridge that is healthy locally and unreachable publicly is the
//!    failure this check exists to surface.
//!
//! Every number in the panel names its question (AC-6): `bridge /health: last answer 2s ago` beats
//! a green dot, and `edge /health: 502 — <cause>` beats a red one.

use std::time::{Duration, Instant};

use crate::http::{Http, HEALTH_TIMEOUT};
use crate::runner::CommandRunner;
use crate::secret::Secret;

use super::control::{query, SvcName, TaskState};
use super::pair::hub_origin;
use super::Failure;

/// How long a whole status sweep may take before the panel says so.
pub const STATUS_BUDGET: Duration = Duration::from_secs(6);

/// A `/health` answer, kept as raw JSON text so the panel can show a field without this crate
/// re-declaring the bridge's contract (D-032 freezes the bridge's surface, not ours to mirror).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct HealthAnswer {
    pub status: u16,
    pub body: String,
    /// How long the answer took. Shown as "answered in Nms" — a bridge that takes 3 s is a different
    /// problem from one that answers instantly.
    pub elapsed_ms: u64,
}

impl HealthAnswer {
    /// The one field of the bridge's `/health` a human reads first: the MT5 server name.
    pub fn server(&self) -> Option<String> {
        serde_json::from_str::<serde_json::Value>(&self.body)
            .ok()
            .and_then(|v| v.get("server").and_then(|s| s.as_str()).map(str::to_string))
    }

    pub fn ok(&self) -> bool {
        self.status == 200
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskReport {
    pub name: &'static str,
    pub state: TaskState,
    /// Set when the query itself failed (e.g. access denied). The state is then unknown, and the
    /// panel says that rather than showing "not registered".
    pub error: Option<String>,
}

/// The public edge's answer. Separate from [`HealthAnswer`] because a transport failure there is
/// routine (a tunnel warming up) and must not read as a bridge defect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdgeHealth {
    Answered(HealthAnswer),
    Unreachable(String),
    /// No edge URL configured: the app never guessed one.
    NotConfigured,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusReport {
    pub local: Result<HealthAnswer, String>,
    pub bridge_task: TaskReport,
    pub tunnel_task: TaskReport,
    pub edge: EdgeHealth,
    /// Total time the sweep took, so the panel can say when it was measured.
    pub elapsed_ms: u64,
}

/// Where the app looks. `bridge_port` defaults to the host installer's 8797
/// (`bridge/host-install.ps1:83`).
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub bridge_port: u16,
    /// The public URL the hub registers; `None` until the tunnel URL is known.
    pub edge_url: Option<String>,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            bridge_port: 8797,
            edge_url: None,
        }
    }
}

/// The local bridge's /health. Loopback plus the bearer header, bounded.
pub fn local_health<H: Http>(http: &H, port: u16, token: &Secret) -> Result<HealthAnswer, String> {
    let url = format!("http://127.0.0.1:{port}/health");
    let started = Instant::now();
    let bearer = format!("Bearer {}", token.expose());
    match http.get(&url, Some(&bearer), HEALTH_TIMEOUT) {
        Ok(r) => Ok(HealthAnswer {
            status: r.status,
            body: r.body,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }),
        Err(e) => Err(match e {
            crate::http::HttpError::Timeout => {
                format!("no answer within {}s", HEALTH_TIMEOUT.as_secs())
            }
            crate::http::HttpError::Transport(t) => t,
        }),
    }
}

/// The public edge's /health: the URL in the hub's bridge row, not 127.0.0.1 (AC-4's last clause).
pub fn edge_health<H: Http>(http: &H, edge_url: &str, token: &Secret) -> EdgeHealth {
    let trimmed = edge_url.trim();
    if trimmed.is_empty() {
        return EdgeHealth::NotConfigured;
    }
    // An http:// public edge is refused for the same reason pair refuses one: the token would
    // cross in clear text. The tunnel is https-only in practice.
    if let Err(e) = https_only(trimmed) {
        return EdgeHealth::Unreachable(e.cause());
    }
    let url = format!("{}/health", trimmed.trim_end_matches('/'));
    let started = Instant::now();
    let bearer = format!("Bearer {}", token.expose());
    match http.get(&url, Some(&bearer), HEALTH_TIMEOUT) {
        Ok(r) => EdgeHealth::Answered(HealthAnswer {
            status: r.status,
            body: r.body,
            elapsed_ms: started.elapsed().as_millis() as u64,
        }),
        Err(e) => EdgeHealth::Unreachable(e.cause()),
    }
}

fn https_only(raw: &str) -> Result<(), Failure> {
    let parsed = url::Url::parse(raw)
        .map_err(|e| Failure::Input(format!("the edge URL does not parse: {e}")))?;
    if parsed.scheme() != "https" {
        return Err(Failure::Input(format!(
            "the edge URL must be https://, not {0}://",
            parsed.scheme()
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Failure::Input(
            "the edge URL must not carry credentials".to_string(),
        ));
    }
    Ok(())
}

/// The whole sweep. A failing part is reported, never fatal: the panel must show what it knows.
pub fn status<H: Http, R: CommandRunner>(
    http: &H,
    runner: &R,
    endpoints: &Endpoints,
    token: &Secret,
) -> StatusReport {
    let started = Instant::now();
    let local = local_health(http, endpoints.bridge_port, token);
    let bridge_task = task_report(runner, SvcName::Bridge);
    let tunnel_task = task_report(runner, SvcName::Tunnel);
    let edge = match &endpoints.edge_url {
        Some(u) => edge_health(http, u, token),
        None => EdgeHealth::NotConfigured,
    };
    StatusReport {
        local,
        bridge_task,
        tunnel_task,
        edge,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }
}

fn task_report<R: CommandRunner>(runner: &R, svc: SvcName) -> TaskReport {
    match query(runner, svc) {
        Ok(state) => TaskReport {
            name: svc.task_name(),
            state,
            error: None,
        },
        Err(e) => TaskReport {
            name: svc.task_name(),
            state: TaskState::Missing,
            error: Some(e.cause()),
        },
    }
}

/// Normalises a user-typed hub URL for the status view's "paired with" line. Shares pair's rule,
/// so the panel cannot show one origin while pair posts to another.
pub fn normalized_hub(hub_url: &str) -> Result<String, Failure> {
    hub_origin(hub_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Fake, HttpError, HttpResponse};
    use crate::runner::{CmdResult, Fake as FakeRunner};

    fn token() -> Secret {
        Secret::new("tok".to_string())
    }

    fn running() -> CmdResult {
        CmdResult::success("Status:        Running\r\n")
    }

    fn ready() -> CmdResult {
        CmdResult::success("Status:        Ready\r\n")
    }

    fn health_body() -> String {
        r#"{"ok":true,"server":"Exness-MT5Real28","pollers":1,"offset_s":0.4,"offset_src":"terminal","measured_at":1795000000.5}"#.to_string()
    }

    #[test]
    fn the_local_health_uses_loopback_and_the_bearer_header() {
        let http = Fake::with_gets(vec![Ok(HttpResponse {
            status: 200,
            body: health_body(),
        })]);
        let a = local_health(&http, 8797, &token()).expect("answers");
        assert!(a.ok());
        assert_eq!(a.server().as_deref(), Some("Exness-MT5Real28"));
        let calls = http.get_calls();
        assert_eq!(calls[0].0, "http://127.0.0.1:8797/health");
        assert_eq!(calls[0].1.as_deref(), Some("Bearer tok"));
    }

    #[test]
    fn a_timed_out_local_health_names_the_deadline() {
        let http = Fake::with_gets(vec![Err(HttpError::Timeout)]);
        let err = local_health(&http, 8797, &token()).expect_err("timeout");
        assert_eq!(err, "no answer within 4s");
    }

    #[test]
    fn the_edge_check_hits_the_tunnel_url_not_loopback() {
        let http = Fake::with_gets(vec![Ok(HttpResponse {
            status: 200,
            body: health_body(),
        })]);
        let e = edge_health(&http, "https://mt5.vtradingview.com/", &token());
        assert!(matches!(e, EdgeHealth::Answered(_)));
        let calls = http.get_calls();
        assert_eq!(
            calls[0].0, "https://mt5.vtradingview.com/health",
            "loopback here would prove nothing about the internet"
        );
        assert!(!calls[0].0.contains("127.0.0.1"));
    }

    #[test]
    fn an_unreachable_edge_names_its_cause_and_is_not_a_bridge_defect() {
        let http = Fake::with_gets(vec![Err(HttpError::Transport("dns error".to_string()))]);
        let e = edge_health(&http, "https://x.example", &token());
        assert_eq!(e, EdgeHealth::Unreachable("dns error".to_string()));
    }

    #[test]
    fn an_edge_url_with_credentials_is_refused_before_the_request() {
        let http = Fake::default();
        let e = edge_health(&http, "https://user:pw@x.example", &token());
        assert!(
            matches!(e, EdgeHealth::Unreachable(ref c) if c.contains("must not carry credentials"))
        );
        assert_eq!(http.total_calls(), 0);
    }

    #[test]
    fn an_http_edge_url_is_refused_without_a_request() {
        let http = Fake::default();
        let e = edge_health(&http, "http://x.example", &token());
        assert!(matches!(e, EdgeHealth::Unreachable(ref c) if c.contains("must be https://")));
        assert_eq!(http.total_calls(), 0);
    }

    #[test]
    fn no_edge_url_is_reported_as_not_configured_not_as_down() {
        let http = Fake::default();
        assert_eq!(
            edge_health(&http, "  ", &token()),
            EdgeHealth::NotConfigured
        );
    }

    #[test]
    fn a_full_sweep_reports_all_four_answers_even_when_the_bridge_is_down() {
        // local /health times out; edge answers 200; both tasks Running.
        let http = Fake::with_gets(vec![
            Err(HttpError::Timeout),
            Ok(HttpResponse {
                status: 200,
                body: health_body(),
            }),
        ]);
        let runner = FakeRunner::new(vec![running(), running()]);
        let ep = Endpoints {
            bridge_port: 8797,
            edge_url: Some("https://edge.example".to_string()),
        };
        let r = status(&http, &runner, &ep, &token());
        assert!(r.local.is_err(), "the local half failed as scripted");
        assert_eq!(r.bridge_task.state, TaskState::Running);
        assert_eq!(r.tunnel_task.state, TaskState::Running);
        assert!(matches!(r.edge, EdgeHealth::Answered(_)));
    }

    #[test]
    fn a_task_query_error_is_reported_as_an_error_not_as_missing() {
        let http = Fake::with_gets(vec![Err(HttpError::Timeout), Err(HttpError::Timeout)]);
        let runner = FakeRunner::new(vec![
            CmdResult::failure(5, "ERROR: Access is denied.\r\n"),
            ready(),
        ]);
        let r = status(&http, &runner, &Endpoints::default(), &token());
        assert_eq!(
            r.bridge_task.error.as_deref(),
            Some("ERROR: Access is denied.")
        );
        assert!(r.tunnel_task.error.is_none());
        assert_eq!(r.tunnel_task.state, TaskState::Ready);
    }

    #[test]
    fn the_panel_never_needs_an_edge_url_to_show_a_status() {
        let http = Fake::with_gets(vec![Ok(HttpResponse {
            status: 200,
            body: health_body(),
        })]);
        let runner = FakeRunner::new(vec![ready(), ready()]);
        let r = status(&http, &runner, &Endpoints::default(), &token());
        assert_eq!(r.edge, EdgeHealth::NotConfigured);
        assert!(r.local.is_ok());
    }

    #[test]
    fn a_non_200_local_answer_is_reported_with_its_status_and_body() {
        let http = Fake::with_gets(vec![Ok(HttpResponse {
            status: 401,
            body: r#"{"error":"unauthorized"}"#.to_string(),
        })]);
        let a = local_health(&http, 8797, &token()).expect("an answer, not a transport failure");
        assert!(!a.ok());
        assert_eq!(a.status, 401);
        assert!(a.body.contains("unauthorized"));
    }

    #[test]
    fn the_hub_url_normalisation_is_the_same_rule_pair_uses() {
        assert_eq!(
            normalized_hub("https://vtradingview.com/").expect("ok"),
            "https://vtradingview.com"
        );
        assert!(normalized_hub("http://vtradingview.com").is_err());
    }
}
