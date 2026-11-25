//! The three free providers. One file per provider holds all of that provider's knowledge; upstream
//! JSON is parsed by private serde DTOs inside each file and never leaves it.
//!
//! 01-01 owns the REST halves (symbols, history); 01-02 adds the live halves.

pub mod binance;
pub mod bybit;
pub mod deriv;

use crate::market::candles::{FeedRules, TickBatch};
use crate::market::{BarsQuery, FeedError, Ticker};
use futures_util::future::BoxFuture;
use hub_types::market::{Bar, Source, SymbolEntry};
use std::time::Duration;

/// Upstream base URLs. Tests point every provider at local fakes with `AppState::build_with`.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub binance_rest: String,
    pub bybit_rest: String,
    pub deriv_ws: String,
    /// Live trades on port 443 (Python found 9443 blocked).
    pub binance_ws: String,
    /// Live trades; the category (linear, spot, inverse) is appended.
    pub bybit_ws: String,
    /// Binance USD-M futures REST (order flow: klines, aggTrades, exchangeInfo).
    pub binance_fapi: String,
    /// Binance public data archive (order flow: daily aggTrades files).
    pub binance_vision: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Endpoints {
            binance_rest: "https://api.binance.com".into(),
            bybit_rest: "https://api.bybit.com".into(),
            deriv_ws: deriv::DERIV_URL.into(),
            binance_ws: "wss://data-stream.binance.vision".into(),
            bybit_ws: "wss://stream.bybit.com/v5/public/".into(),
            binance_fapi: "https://fapi.binance.com".into(),
            binance_vision: "https://data.binance.vision".into(),
        }
    }
}

/// Fetches history for one normalized query. `Sources` is the real implementation; tests count
/// calls with a fake.
pub trait HistoryFetch: Send + Sync + 'static {
    fn fetch<'a>(&'a self, q: &'a BarsQuery) -> BoxFuture<'a, Result<Vec<Bar>, FeedError>>;
}

/// The providers' REST side, shared by `/api/bars`, the symbol refresh loops and (01-02) the
/// websocket history.
pub struct Sources {
    http: reqwest::Client,
    endpoints: Endpoints,
    deriv: deriv::RpcPool,
}

impl Sources {
    pub fn new(endpoints: Endpoints) -> Sources {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("tradex-hub")
            .build()
            .expect("reqwest client builds with static settings");
        let deriv = deriv::RpcPool::new(endpoints.deriv_ws.clone());
        Sources {
            http,
            endpoints,
            deriv,
        }
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    /// History for one query, dispatched by source.
    pub async fn history(&self, q: &BarsQuery) -> Result<Vec<Bar>, FeedError> {
        match q.source {
            Source::Binance => binance::history(&self.http, &self.endpoints.binance_rest, q).await,
            Source::Bybit => bybit::history(&self.http, &self.endpoints.bybit_rest, q).await,
            Source::Deriv => deriv::history(&self.deriv, q).await,
        }
    }

    /// Every trading Bybit market, most traded first inside each group.
    pub async fn bybit_symbols(&self) -> Result<Vec<SymbolEntry>, FeedError> {
        bybit::symbols(&self.http, &self.endpoints.bybit_rest).await
    }

    /// The Deriv symbol list.
    pub async fn deriv_symbols(&self) -> Result<Vec<SymbolEntry>, FeedError> {
        deriv::symbols(&self.deriv).await
    }
}

impl HistoryFetch for Sources {
    fn fetch<'a>(&'a self, q: &'a BarsQuery) -> BoxFuture<'a, Result<Vec<Bar>, FeedError>> {
        Box::pin(self.history(q))
    }
}

/// GET a URL and decode JSON, with Python's error wording for an HTTP status
/// (`HTTP Error 400: Bad Request`).
pub(crate) async fn get_json<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    url: &str,
) -> Result<T, FeedError> {
    let resp = http
        .get(url)
        .send()
        .await
        .map_err(|e| FeedError::new(transport_text(&e)))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(FeedError::new(format!(
            "HTTP Error {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        )));
    }
    resp.json::<T>()
        .await
        .map_err(|e| FeedError::new(format!("bad response: {e}")))
}

fn transport_text(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".to_string()
    } else {
        format!("<urlopen error {e}>")
    }
}

/// The identity of one upstream connection: one per `(source, resolved symbol)` (D-002). Binance is
/// keyed by the mapped symbol, so `BTCUSD` and `BTCUSDT` share one `@aggTrade` stream; Bybit by the
/// full hub ticker (category and kind included); Deriv by its ticker.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UpstreamKey {
    pub source: Source,
    pub symbol: Ticker,
}

impl std::fmt::Display for UpstreamKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.source, self.symbol)
    }
}

/// How an upstream session keeps itself alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepAlive {
    /// The websocket library's protocol pings (Binance, Deriv).
    Protocol,
    /// Bybit: library pings off, a JSON `{"op":"ping"}` every interval.
    JsonPing { every: Duration },
}

/// One provider's live session description: data, not code paths.
#[derive(Clone, Debug)]
pub struct LivePlan {
    pub url: String,
    /// Text frames sent right after connecting (subscribe requests).
    pub hello: Vec<String>,
    pub keepalive: KeepAlive,
    pub rules: FeedRules,
    /// The wait after an `EndSession` reply (Deriv market closed: 60 s, as in Python).
    pub end_session_wait: Duration,
}

/// What one upstream text frame means.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Batch(TickBatch),
    /// Acks, pongs, other topics, deltas without the field.
    Ignore,
    /// The provider ended the session (Deriv `error` reply); `status` is the browser notice.
    EndSession {
        status: String,
    },
}

/// The live plan for an upstream.
pub fn live_plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan {
    match key.source {
        Source::Binance => binance::live_plan(key, ep),
        Source::Bybit => bybit::live_plan(key, ep),
        Source::Deriv => deriv::live_plan(key, ep),
    }
}

/// Parses one upstream text frame for `key`. Pure.
pub fn parse_frame(key: &UpstreamKey, text: &str) -> Parsed {
    match key.source {
        Source::Binance => binance::parse_frame(text),
        Source::Bybit => bybit::parse_frame(key.symbol.as_str(), text),
        Source::Deriv => deriv::parse_frame(text),
    }
}

/// The default chart every visitor lands on (`hub/hub.html`: deriv `frxXAUUSD`).
///
/// The feed cap reserves this key, so the first thing a new visitor sees can never be refused with
/// "too many markets are open". It is the one key the cap may force-evict a live upstream for, and
/// it is never chosen as an idle victim.
///
/// `None` only if the constant stops parsing, which a unit test pins: a reserve that silently
/// became `None` would be a cap with no reservation and no failure to notice it.
pub fn reserved_key() -> Option<UpstreamKey> {
    Some(UpstreamKey {
        source: Source::Deriv,
        symbol: Ticker::parse(Source::Deriv, "frxXAUUSD").ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reserved_key_is_the_default_chart() {
        let k = reserved_key().expect("the default chart parses");
        assert_eq!(k.source, Source::Deriv);
        assert_eq!(k.symbol.as_str(), "frxXAUUSD");
        // A different Deriv ticker is a different key: the reserve is exactly one upstream.
        assert_ne!(
            reserved_key(),
            Some(UpstreamKey {
                source: Source::Deriv,
                symbol: Ticker::parse(Source::Deriv, "frxEURUSD").unwrap(),
            })
        );
    }
}
