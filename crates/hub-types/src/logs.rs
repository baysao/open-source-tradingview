//! `GET /api/logs` (`hub/logbus.py:243-256` plus `journal_raw`). Read by `hub/log_panel.js`.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One log line. `t` is SNTP-corrected UTC ms; `lvl` is debug, info, warn or error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRow {
    pub id: u64,
    pub t: i64,
    pub src: String,
    pub lvl: String,
    pub msg: String,
}

/// What the panel knows about one log source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInfo {
    /// Newest line or file write, ms.
    pub last: i64,
    /// Expected interval in seconds; never set in this build (Python never called `expect`).
    pub expect: Option<u64>,
    /// `ok` or `silent`.
    pub state: String,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogSnapshot {
    /// Process start, epoch seconds; the panel resets when it changes.
    pub boot: i64,
    /// Newest line id.
    pub last: u64,
    /// Now, ms.
    pub now: i64,
    pub errors_15m: usize,
    pub latest_error: Option<LogRow>,
    pub problems: BTreeMap<String, String>,
    pub sources: BTreeMap<String, SourceInfo>,
    /// Absent with `brief`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<LogRow>>,
}

/// The response: the snapshot plus `journal_raw`, always false (the panel never reads it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogsResponse {
    #[serde(flatten)]
    pub snapshot: LogSnapshot,
    pub journal_raw: bool,
}
