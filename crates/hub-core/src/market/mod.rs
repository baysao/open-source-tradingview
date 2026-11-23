//! Pure market rules: no I/O, no clock reads, no `.await`.
//!
//! 01-01 owns `ticker`, `bars_query`, `instruments`; 01-02 adds `candles`, `feed_health`, `backoff`.

pub mod backoff;
pub mod bars_query;
pub mod candles;
pub mod feed_health;
pub mod instruments;
pub mod ticker;

pub use bars_query::BarsQuery;
pub use ticker::Ticker;

use std::fmt;

/// An error a feed request ends with. Its text is what `/api/bars` returns as `{"error": ...}`
/// with status 502 (Python answers every failure there with 502, `hub/server.py:983`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedError(pub String);

impl FeedError {
    pub fn new(msg: impl Into<String>) -> FeedError {
        FeedError(msg.into())
    }
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FeedError {}
