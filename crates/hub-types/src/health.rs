//! `GET /api/health` (`hub/server.py:545-552`). No page reads it; it is the "agent" contract.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Freshness and lag of one upstream feed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedHealthEntry {
    pub ticks: u64,
    pub age_ms: i64,
    pub lag_median_ms: Option<i64>,
    pub lag_p95_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthResponse {
    pub clock_offset_ms: i64,
    /// Keyed `"<source>:<symbol>"`; one entry per live upstream (Binance by the mapped symbol).
    pub feeds: BTreeMap<String, FeedHealthEntry>,
}
