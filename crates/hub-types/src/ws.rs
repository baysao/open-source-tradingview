//! `/ws` messages, kept exactly as `hub/server.py` sends them (D-002).
//!
//! One subscription per socket: the browser sends one untyped `{source, symbol, tf}`; a new message
//! replaces the stream. The server answers `history`, `bar`, `status`, `error`; the only addition is
//! the optional `kind: "capacity"` on `error`.

use crate::market::Bar;
use serde::{Deserialize, Serialize};

/// What the browser sends. Fields are untyped strings as in Python (`req.get(...)` with defaults
/// binance, XAUUSD, M1); unknown fields are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscribe {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default)]
    pub tf: Option<String>,
}

/// Why a subscription was refused, when it is not a plain message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    /// The server already holds `FEED_MAX_SYMBOLS` upstreams (D-002).
    Capacity,
}

/// What the server sends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ServerMsg {
    /// Sent once per subscription. Binance reports the mapped symbol (PAXGUSDT for XAUUSD); Bybit
    /// and Deriv echo the request.
    History {
        bars: Vec<Bar>,
        source: String,
        symbol: String,
        tf: String,
    },
    /// The current bar after an update. `trade_ms` only for Binance and Bybit; `sent` is the local
    /// clock, `lag_ms` is measured against the SNTP-corrected clock.
    Bar {
        bar: Bar,
        sent: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trade_ms: Option<i64>,
        lag_ms: Option<f64>,
    },
    /// A reconnect or market-closed notice.
    Status { msg: String },
    /// A failure; the socket stays open.
    Error {
        msg: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ErrorKind>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shapes() {
        let bar = Bar {
            time: 60,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            volume: 3.0,
        };
        let deriv = ServerMsg::Bar {
            bar,
            sent: 1,
            trade_ms: None,
            lag_ms: Some(12.5),
        };
        let v = serde_json::to_value(&deriv).unwrap();
        assert_eq!(v["type"], "bar");
        assert!(v.get("trade_ms").is_none(), "Deriv bars carry no trade_ms");
        let err = ServerMsg::Error {
            msg: "full".into(),
            kind: Some(ErrorKind::Capacity),
        };
        assert_eq!(
            serde_json::to_string(&err).unwrap(),
            r#"{"type":"error","msg":"full","kind":"capacity"}"#
        );
        let plain = ServerMsg::Error {
            msg: "Unknown timeframe M2".into(),
            kind: None,
        };
        assert_eq!(
            serde_json::to_string(&plain).unwrap(),
            r#"{"type":"error","msg":"Unknown timeframe M2"}"#
        );
        let sub: Subscribe =
            serde_json::from_str(r#"{"source":"bybit","symbol":"BYBIT_BTCUSDT","tf":"M1","x":1}"#)
                .unwrap();
        assert_eq!(sub.tf.as_deref(), Some("M1"));
    }
}
