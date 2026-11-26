//! The view types the panel reads: pure data, no tauri.
//!
//! Split from `app.rs` so they are compiled and tested WITHOUT the `gui` feature: the JSON shape
//! the UI depends on is the part most likely to drift, and it must be covered by the plain
//! `cargo test` that CI runs on a machine with no webkit (AC-1).

use crate::commands::control::TaskState;
use crate::commands::status::{EdgeHealth, TaskReport};
use crate::commands::Failure;

/// A refusal the UI can render: the cause, and the next action when one exists.
#[derive(Debug, serde::Serialize)]
pub struct CommandError {
    pub cause: String,
    pub next_action: Option<String>,
}

impl From<Failure> for CommandError {
    fn from(f: Failure) -> Self {
        Self {
            cause: f.cause(),
            next_action: f.next_action().map(str::to_string),
        }
    }
}

impl CommandError {
    pub fn new(cause: impl Into<String>, next_action: Option<&str>) -> Self {
        Self {
            cause: cause.into(),
            next_action: next_action.map(str::to_string),
        }
    }
}

/// The pair result as the page sees it. `revoke_code` is the ONE moment the secret crosses the IPC
/// boundary; the page shows it once, offers a copy gesture, and drops it.
#[derive(Debug, serde::Serialize)]
pub struct PairView {
    pub id: String,
    pub label: String,
    pub url: String,
    pub symbol_count: u64,
    pub connected: bool,
    pub revoke_code: String,
}

/// One task's line: the task name, what state it is in, and the cause when the query itself failed.
pub fn task_line(t: &TaskReport) -> String {
    match &t.error {
        Some(e) => format!("{}: unknown ({})", t.name, e),
        None => format!(
            "{}: {}",
            t.name,
            match t.state {
                TaskState::Running => "running",
                TaskState::Ready => "registered, not running",
                TaskState::Missing => "not registered",
            }
        ),
    }
}

/// The status the panel renders. Every field names its question (AC-6):
/// `local_detail` says what answered and how fast, `edge_detail` says whether the INTERNET sees the
/// bridge, and the two task lines say whether the services are registered and running.
#[derive(Debug, serde::Serialize)]
pub struct StatusView {
    pub local_ok: bool,
    pub local_detail: String,
    pub bridge_task: String,
    pub tunnel_task: String,
    pub edge_ok: bool,
    pub edge_detail: String,
    /// Total sweep time, so the panel can date its own numbers.
    pub elapsed_ms: u64,
}

impl StatusView {
    pub fn from_report(r: &crate::commands::StatusReport) -> Self {
        Self {
            local_ok: r.local.as_ref().map(|a| a.ok()).unwrap_or(false),
            local_detail: match &r.local {
                Ok(a) => format!(
                    "answered {} in {}ms{}",
                    a.status,
                    a.elapsed_ms,
                    a.server().map(|s| format!(" — {s}")).unwrap_or_default()
                ),
                Err(c) => c.clone(),
            },
            bridge_task: task_line(&r.bridge_task),
            tunnel_task: task_line(&r.tunnel_task),
            edge_ok: matches!(&r.edge, EdgeHealth::Answered(a) if a.ok()),
            edge_detail: match &r.edge {
                EdgeHealth::Answered(a) => format!("answered {} in {}ms", a.status, a.elapsed_ms),
                EdgeHealth::Unreachable(c) => c.clone(),
                EdgeHealth::NotConfigured => "not configured yet".to_string(),
            },
            elapsed_ms: r.elapsed_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::status::{HealthAnswer, StatusReport};

    fn report_local_ok() -> StatusReport {
        StatusReport {
            local: Ok(HealthAnswer {
                status: 200,
                body: r#"{"ok":true,"server":"Exness-MT5Real28"}"#.to_string(),
                elapsed_ms: 3,
            }),
            bridge_task: TaskReport {
                name: "VTVBridge",
                state: TaskState::Running,
                error: None,
            },
            tunnel_task: TaskReport {
                name: "VTVTunnel",
                state: TaskState::Missing,
                error: None,
            },
            edge: EdgeHealth::Answered(HealthAnswer {
                status: 200,
                body: "{}".to_string(),
                elapsed_ms: 40,
            }),
            elapsed_ms: 44,
        }
    }

    #[test]
    fn the_status_view_names_the_question_each_number_answers() {
        let v = StatusView::from_report(&report_local_ok());
        assert!(v.local_ok);
        assert_eq!(v.local_detail, "answered 200 in 3ms — Exness-MT5Real28");
        assert_eq!(v.bridge_task, "VTVBridge: running");
        assert_eq!(v.tunnel_task, "VTVTunnel: not registered");
        assert!(v.edge_ok);
        assert_eq!(v.elapsed_ms, 44);
    }

    #[test]
    fn a_local_failure_carries_its_cause_into_the_view() {
        let mut r = report_local_ok();
        r.local = Err("no answer within 4s".to_string());
        let v = StatusView::from_report(&r);
        assert!(!v.local_ok);
        assert_eq!(v.local_detail, "no answer within 4s");
    }

    #[test]
    fn an_unqueryable_task_says_unknown_not_not_registered() {
        let line = task_line(&TaskReport {
            name: "VTVBridge",
            state: TaskState::Missing,
            error: Some("ERROR: Access is denied.".to_string()),
        });
        assert!(line.contains("unknown"), "{line}");
        assert!(!line.contains("not registered"), "{line}");
    }

    #[test]
    fn an_unreachable_edge_is_reported_verbatim() {
        let mut r = report_local_ok();
        r.edge = EdgeHealth::Unreachable("dns error".to_string());
        let v = StatusView::from_report(&r);
        assert!(!v.edge_ok);
        assert_eq!(v.edge_detail, "dns error");
    }

    #[test]
    fn a_failure_becomes_a_cause_and_a_next_action_for_the_ui() {
        let e: CommandError = Failure::Hub {
            status: 401,
            cause: "code unknown or expired".to_string(),
        }
        .into();
        assert_eq!(e.cause, "code unknown or expired");
        assert!(e.next_action.unwrap_or_default().contains("fresh one"));
    }

    #[test]
    fn the_view_serializes_with_the_field_names_the_page_reads() {
        let v = StatusView::from_report(&report_local_ok());
        let json = serde_json::to_value(&v).expect("serialises");
        for key in [
            "local_ok",
            "local_detail",
            "bridge_task",
            "tunnel_task",
            "edge_ok",
            "edge_detail",
            "elapsed_ms",
        ] {
            assert!(json.get(key).is_some(), "manager.js reads .{key}");
        }
    }
}
