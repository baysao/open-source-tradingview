//! `GET /api/bars` parameters, parsed once at the boundary.

use super::{FeedError, Ticker};
use hub_types::market::{Source, Tf};
use std::collections::HashMap;

/// Python's `HISTORY` (`hub/server.py:58`): the default `limit`.
pub const DEFAULT_LIMIT: i64 = 1000;

/// A normalized history request. It is the `BarsCache` key, so two requests that ask the
/// upstream for the same thing compare equal: Binance symbols are mapped, and Binance and Bybit
/// limits are capped at 1,000 (both APIs return at most 1,000 rows, no pagination).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BarsQuery {
    pub source: Source,
    pub symbol: Ticker,
    pub tf: Tf,
    pub limit: i64,
    /// Newest bar wanted, epoch milliseconds (scroll-back).
    pub to_ms: Option<i64>,
}

impl BarsQuery {
    /// Builds a query from the raw query string pairs. Defaults: Binance, `BTCUSD`, `M1`, 1,000.
    /// An unknown source means Binance. An unknown tf fails with Python's `KeyError` text
    /// (`'M2'`), which `/api/bars` returns with status 502.
    pub fn from_params(p: &HashMap<String, String>) -> Result<BarsQuery, FeedError> {
        let get = |k: &str| p.get(k).map(String::as_str).filter(|v| !v.is_empty());
        let source = Source::from_wire(get("source"));
        let tf_raw = get("tf").unwrap_or("M1");
        let tf = Tf::parse(tf_raw).ok_or_else(|| FeedError::new(format!("'{tf_raw}'")))?;
        let limit = match get("limit") {
            Some(v) => v.trim().parse::<i64>().map_err(|_| {
                FeedError::new(format!("invalid literal for int() with base 10: '{v}'"))
            })?,
            None => DEFAULT_LIMIT,
        };
        let to_ms = match get("to") {
            Some(v) => Some(v.trim().parse::<f64>().map(|f| f as i64).map_err(|_| {
                FeedError::new(format!("invalid literal for int() with base 10: '{v}'"))
            })?),
            None => None,
        };
        let symbol = Ticker::parse(source, get("symbol").unwrap_or("BTCUSD"))?;
        Ok(BarsQuery::new(source, symbol, tf, limit, to_ms))
    }

    /// A normalized query (Binance and Bybit `limit` capped at 1,000).
    pub fn new(
        source: Source,
        symbol: Ticker,
        tf: Tf,
        limit: i64,
        to_ms: Option<i64>,
    ) -> BarsQuery {
        let limit = match source {
            Source::Binance | Source::Bybit => limit.min(1000),
            Source::Deriv => limit,
        };
        BarsQuery {
            source,
            symbol,
            tf,
            limit,
            to_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(pairs: &[(&str, &str)]) -> Result<BarsQuery, FeedError> {
        BarsQuery::from_params(
            &pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn defaults_and_fallbacks() {
        let d = q(&[]).unwrap();
        assert_eq!(
            (d.source, d.symbol.as_str(), d.tf, d.limit, d.to_ms),
            (Source::Binance, "BTCUSDT", Tf::M1, 1000, None)
        );
        assert_eq!(q(&[("source", "nope")]).unwrap().source, Source::Binance);
        assert_eq!(q(&[("tf", "M2")]).unwrap_err().0, "'M2'");
    }

    #[test]
    fn normalization_makes_equal_keys() {
        let a = q(&[("symbol", "BTCUSD"), ("limit", "5000")]).unwrap();
        let b = q(&[("symbol", "btcusdt"), ("limit", "1000")]).unwrap();
        assert_eq!(a, b);
        let deriv = q(&[
            ("source", "deriv"),
            ("symbol", "frxXAUUSD"),
            ("limit", "5000"),
        ])
        .unwrap();
        assert_eq!(deriv.limit, 5000);
    }

    #[test]
    fn to_is_epoch_ms_and_blank_is_none() {
        assert_eq!(
            q(&[("to", "1700000000000")]).unwrap().to_ms,
            Some(1_700_000_000_000)
        );
        assert_eq!(q(&[("to", "")]).unwrap().to_ms, None);
        assert!(q(&[("limit", "abc")]).is_err());
    }
}
