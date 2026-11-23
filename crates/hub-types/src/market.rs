//! Market HTTP shapes: `/api/status`, `/api/symbols`, `/api/instruments`, `/api/bars`.
//!
//! Field names, order and presence follow what `hub/server.py` sends with Local MT5 and Remote MT5
//! off (D-001, D-003). `deny_unknown_fields` is on every struct so a test that deserializes Python
//! output fails loudly on a field this model does not know.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::fmt;

/// The three free feeds. An unknown or missing source means Binance on `/api/bars` and `/ws`
/// (`hub/server.py:697`, `979`); `from_wire` is the one place that rule lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Deriv,
    Bybit,
    Binance,
}

impl Source {
    pub const ALL: [Source; 3] = [Source::Deriv, Source::Bybit, Source::Binance];

    /// `None`, `"mt5"`, `"arm"` and any unknown string become `Binance` (Python parity).
    pub fn from_wire(s: Option<&str>) -> Source {
        match s {
            Some("deriv") => Source::Deriv,
            Some("bybit") => Source::Bybit,
            _ => Source::Binance,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Source::Deriv => "deriv",
            Source::Bybit => "bybit",
            Source::Binance => "binance",
        }
    }

    /// Display name used in `/api/instruments` brokers and in error texts.
    pub fn display(self) -> &'static str {
        match self {
            Source::Deriv => "Deriv",
            Source::Bybit => "Bybit",
            Source::Binance => "Binance",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The only valid timeframes (`hub/server.py:49`). Serialized as "M1".."D1".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tf {
    M1,
    M5,
    M15,
    M30,
    H1,
    H4,
    D1,
}

impl Tf {
    pub const ALL: [Tf; 7] = [Tf::M1, Tf::M5, Tf::M15, Tf::M30, Tf::H1, Tf::H4, Tf::D1];

    pub fn parse(s: &str) -> Option<Tf> {
        Tf::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// 60, 300, 900, 1800, 3600, 14400, 86400.
    pub fn secs(self) -> i64 {
        match self {
            Tf::M1 => 60,
            Tf::M5 => 300,
            Tf::M15 => 900,
            Tf::M30 => 1800,
            Tf::H1 => 3600,
            Tf::H4 => 14400,
            Tf::D1 => 86400,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tf::M1 => "M1",
            Tf::M5 => "M5",
            Tf::M15 => "M15",
            Tf::M30 => "M30",
            Tf::H1 => "H1",
            Tf::H4 => "H4",
            Tf::D1 => "D1",
        }
    }
}

impl fmt::Display for Tf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One candle. `time` is epoch seconds of the bar open. Deriv volume is always 0.0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// Menu group of a symbol, as Python names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolGroup {
    Commodity,
    Forex,
    Indices,
    Crypto,
    BybitMetal,
    BybitPerp,
    BybitSpot,
    BybitFutures,
    BybitInverse,
    BybitMark,
    BybitIndex,
}

impl SymbolGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolGroup::Commodity => "commodity",
            SymbolGroup::Forex => "forex",
            SymbolGroup::Indices => "indices",
            SymbolGroup::Crypto => "crypto",
            SymbolGroup::BybitMetal => "bybit-metal",
            SymbolGroup::BybitPerp => "bybit-perp",
            SymbolGroup::BybitSpot => "bybit-spot",
            SymbolGroup::BybitFutures => "bybit-futures",
            SymbolGroup::BybitInverse => "bybit-inverse",
            SymbolGroup::BybitMark => "bybit-mark",
            SymbolGroup::BybitIndex => "bybit-index",
        }
    }
}

/// A Deriv entry: `digits` and `open` are always present.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DerivSymbol {
    pub ticker: String,
    pub name: String,
    pub source: Source,
    pub group: SymbolGroup,
    pub digits: u32,
    pub open: bool,
}

/// A Bybit entry: `digits` is always present, there is no `open`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BybitSymbol {
    pub ticker: String,
    pub name: String,
    pub source: Source,
    pub group: SymbolGroup,
    pub digits: u32,
}

/// A Binance entry: no `digits`, no `open` (`hub/server.py:905`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinanceSymbol {
    pub ticker: String,
    pub name: String,
    pub source: Source,
    pub group: SymbolGroup,
}

/// One `/api/symbols` entry. One shape per source, so no field is optional where Python always
/// sends it. Untagged: deserialization tries Deriv, then Bybit, then Binance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SymbolEntry {
    Deriv(DerivSymbol),
    Bybit(BybitSymbol),
    Binance(BinanceSymbol),
}

impl SymbolEntry {
    pub fn ticker(&self) -> &str {
        match self {
            SymbolEntry::Deriv(s) => &s.ticker,
            SymbolEntry::Bybit(s) => &s.ticker,
            SymbolEntry::Binance(s) => &s.ticker,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            SymbolEntry::Deriv(s) => &s.name,
            SymbolEntry::Bybit(s) => &s.name,
            SymbolEntry::Binance(s) => &s.name,
        }
    }

    pub fn source(&self) -> Source {
        match self {
            SymbolEntry::Deriv(_) => Source::Deriv,
            SymbolEntry::Bybit(_) => Source::Bybit,
            SymbolEntry::Binance(_) => Source::Binance,
        }
    }

    pub fn group(&self) -> SymbolGroup {
        match self {
            SymbolEntry::Deriv(s) => s.group,
            SymbolEntry::Bybit(s) => s.group,
            SymbolEntry::Binance(s) => s.group,
        }
    }
}

/// The constant `"off"`: what Python reports for Local MT5 and Remote MT5 when both are disabled.
/// `hub/lab.js:95-97` warns unless the value is "ok" or "off" (D-003).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Off;

impl Serialize for Off {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("off")
    }
}

impl<'de> Deserialize<'de> for Off {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = String::deserialize(d)?;
        if v == "off" {
            Ok(Off)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected \"off\", got {v:?}"
            )))
        }
    }
}

/// `GET /api/symbols`. Entry order: Bybit, Deriv, then the nine Binance entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolsResponse {
    pub symbols: Vec<SymbolEntry>,
    pub mt5: Off,
    pub arm: Off,
    pub deriv: usize,
    pub bybit: usize,
    /// The port the websocket lives on. Python: 8765; Rust: the HTTP port, since `/ws` shares it.
    pub ws_port: u16,
}

/// One status light.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedStatus {
    pub ok: bool,
    pub msg: String,
}

/// `GET /api/status`. `ok` means "symbol list loaded"; Binance is always ok (`hub/server.py:865-870`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusResponse {
    pub deriv: FeedStatus,
    pub bybit: FeedStatus,
    pub binance: FeedStatus,
}

/// One broker in `/api/instruments`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Broker {
    pub id: String,
    pub name: String,
}

/// One market of an instrument at one broker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub ticker: String,
    pub label: String,
}

/// One cross-broker instrument. `group` is the raw symbol group of the first source that named it,
/// `brokers` keeps insertion order (the order the frontend sees).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instrument {
    pub key: String,
    pub name: String,
    pub group: String,
    pub brokers: IndexMap<String, Vec<Variant>>,
}

/// `GET /api/instruments`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentsResponse {
    pub brokers: Vec<Broker>,
    pub instruments: Vec<Instrument>,
}

/// Every API error body: `{"error": "<text>"}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_fallback_is_binance() {
        assert_eq!(Source::from_wire(None), Source::Binance);
        assert_eq!(Source::from_wire(Some("mt5")), Source::Binance);
        assert_eq!(Source::from_wire(Some("nope")), Source::Binance);
        assert_eq!(Source::from_wire(Some("deriv")), Source::Deriv);
    }

    #[test]
    fn tf_table() {
        assert_eq!(Tf::parse("H4").map(Tf::secs), Some(14400));
        assert_eq!(Tf::parse("M2"), None);
        assert_eq!(serde_json::to_string(&Tf::D1).unwrap(), "\"D1\"");
    }

    #[test]
    fn symbol_entries_keep_their_shapes() {
        let deriv = r#"{"ticker":"frxXAUUSD","name":"Gold/USD (Deriv)","source":"deriv","group":"commodity","digits":2,"open":true}"#;
        let bybit = r#"{"ticker":"BYBIT_BTCUSDT","name":"Bybit Perp - BTC/USDT","source":"bybit","group":"bybit-perp","digits":1}"#;
        let binance = r#"{"ticker":"BTCUSD","name":"Bitcoin","source":"binance","group":"crypto"}"#;
        for (raw, src) in [
            (deriv, Source::Deriv),
            (bybit, Source::Bybit),
            (binance, Source::Binance),
        ] {
            let e: SymbolEntry = serde_json::from_str(raw).unwrap();
            assert_eq!(e.source(), src);
            assert_eq!(serde_json::to_string(&e).unwrap(), raw);
        }
    }

    #[test]
    fn off_is_only_off() {
        assert_eq!(serde_json::to_string(&Off).unwrap(), "\"off\"");
        assert!(serde_json::from_str::<Off>("\"ok\"").is_err());
    }
}
