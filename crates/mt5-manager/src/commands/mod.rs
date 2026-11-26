//! The command surface: five domain modules and the error type they share.
//!
//! Each module takes its seams as parameters, so the wiring (`app.rs`) is the only place that
//! names `Real`/`RealFs`, and every test names a recorder.

pub mod control;
pub mod install;
pub mod pair;
pub mod revoke;
pub mod status;

pub use control::{control, ControlOutcome, SvcAction, SvcName, TaskState};
pub use install::{install_stack, InstallOutcome, InstallPaths};
pub use pair::{pair, PairOutcome, PairedBridge};
pub use revoke::revoke;
pub use status::{status, EdgeHealth, StatusReport};

use crate::http::HttpError;
use crate::runner::CmdResult;

/// What went wrong, in the UI's words.
///
/// `cause` is shown verbatim: the hub's refusal texts and the runner's stderr are the exact cause,
/// and paraphrasing one is how an operator ends up chasing the wrong thing (CUJ 7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Bad input caught before any side effect (an http:// hub URL, a malformed code).
    Input(String),
    /// The hub answered, but not with what this call needs.
    Hub { status: u16, cause: String },
    /// The hub was never reached.
    Transport(String),
    /// A local command failed; `cause` is its first stderr line.
    Command { program: String, cause: String },
    /// The local bridge did not answer /health.
    Bridge(String),
    /// A file the app must read or write.
    File(String),
}

impl Failure {
    pub fn cause(&self) -> String {
        match self {
            Failure::Input(c)
            | Failure::Hub { cause: c, .. }
            | Failure::Transport(c)
            | Failure::Command { cause: c, .. }
            | Failure::Bridge(c)
            | Failure::File(c) => c.clone(),
        }
    }

    /// The one sentence the panel shows under the status line: what to do next. Optional because
    /// some failures have no action beyond "read the cause".
    pub fn next_action(&self) -> Option<&'static str> {
        match self {
            Failure::Input(_) => {
                Some("Check the URL starts with https:// and the code is 10 digits.")
            }
            Failure::Hub { status: 401, .. } => Some(
                "The code expired or was already used: mint a fresh one in the hub's Settings.",
            ),
            Failure::Hub { status: 422, .. } => {
                Some("The hub refused this bridge: fix the cause above, then mint a new code.")
            }
            Failure::Hub { .. } => Some("Retry; if it repeats, check the hub's logs."),
            Failure::Transport(_) => Some("Check this host can reach the hub, then retry."),
            Failure::Command { .. } => Some("Check the task name and that this shell is elevated."),
            Failure::Bridge(_) => {
                Some("Is the MT5 terminal running with MCP enabled (Tools > Options > MCP)?")
            }
            Failure::File(_) => Some("Check the path exists and the app may write there."),
        }
    }

    pub fn from_cmd(program: &str, result: &CmdResult) -> Self {
        Failure::Command {
            program: program.to_string(),
            cause: result.cause(),
        }
    }

    pub fn from_http(e: HttpError) -> Self {
        Failure::Transport(e.cause())
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.cause())
    }
}

/// The filesystem seam's errors are already `path: io error` strings, so `?` lands as `File`.
impl From<String> for Failure {
    fn from(s: String) -> Self {
        Failure::File(s)
    }
}

/// The hub's error body is `{"error": "<text>"}` (hub-core/src/routes/mod.rs:51). Falls back to
/// the raw body when a refusal did not come from `api_error`.
pub fn hub_cause(status: u16, body: &str) -> Failure {
    let cause = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                format!("the hub answered {status} with no body")
            } else {
                trimmed.chars().take(200).collect()
            }
        });
    Failure::Hub { status, cause }
}
