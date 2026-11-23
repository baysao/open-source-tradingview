//! A validated upstream symbol.
//!
//! Symbols are pasted into upstream URLs and websocket requests, and the Python server never
//! checked them. `Ticker::parse` applies the Binance mapping first (`bsym_of`, which strips an
//! `EXCHANGE:` prefix and uppercases, `hub/server.py:53-57`), then validates.

use super::FeedError;
use crate::sources::binance::bsym_of;
use hub_types::market::Source;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ticker(String);

impl Ticker {
    /// For Binance the stored symbol is the mapped one (`XAUUSD` becomes `PAXGUSDT`, `BTCUSD`
    /// becomes `BTCUSDT`), so the two spellings of one market share a cache entry and an upstream.
    pub fn parse(source: Source, raw: &str) -> Result<Ticker, FeedError> {
        let candidate = match source {
            Source::Binance => bsym_of(raw),
            Source::Bybit | Source::Deriv => raw.to_string(),
        };
        let valid = (1..=40).contains(&candidate.len())
            && candidate
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-');
        if valid {
            Ok(Ticker(candidate))
        } else {
            Err(FeedError::new(format!("invalid symbol {raw:?}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Ticker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        let ok = [
            (Source::Binance, "BTCUSD", "BTCUSDT"),
            (Source::Binance, "xauusd", "PAXGUSDT"),
            (Source::Binance, "BINANCE:ETHUSD", "ETHUSDT"),
            (Source::Binance, "BTCUSDT", "BTCUSDT"),
            (Source::Deriv, "frxXAUUSD", "frxXAUUSD"),
            (Source::Deriv, "OTC_NDX", "OTC_NDX"),
            (
                Source::Bybit,
                "BYBIT_S_BTCUSDT.MARK",
                "BYBIT_S_BTCUSDT.MARK",
            ),
            (Source::Bybit, "BYBIT_BTC-27MAR26", "BYBIT_BTC-27MAR26"),
        ];
        for (src, raw, want) in ok {
            assert_eq!(Ticker::parse(src, raw).unwrap().as_str(), want, "{raw}");
        }
        let bad = [
            (Source::Deriv, ""),
            (Source::Deriv, "a b"),
            (Source::Bybit, "X/Y"),
            (Source::Deriv, "frx:XAU"),
            (Source::Deriv, &"A".repeat(41)),
        ];
        for (src, raw) in bad {
            assert!(Ticker::parse(src, raw).is_err(), "{raw}");
        }
    }
}
