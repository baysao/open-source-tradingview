//! Binance: spot klines over REST (`hub/server.py:704-714`). The live `@aggTrade` half is 01-02.

use super::get_json;
use crate::market::{BarsQuery, FeedError};
use hub_types::market::{Bar, BinanceSymbol, Source, SymbolEntry, SymbolGroup, Tf};

/// The nine Binance entries of `/api/symbols` (`hub/server.py:716-718`).
pub const CRYPTO: [(&str, &str); 9] = [
    ("BTCUSD", "Bitcoin"),
    ("ETHUSD", "Ethereum"),
    ("SOLUSD", "Solana"),
    ("BNBUSD", "BNB"),
    ("XRPUSD", "XRP"),
    ("DOGEUSD", "Dogecoin"),
    ("ADAUSD", "Cardano"),
    ("LINKUSD", "Chainlink"),
    ("PAXGUSD", "Gold token PAXG (thin trading, not gold spot)"),
];

/// The Binance entries of `/api/symbols`, in order.
pub fn crypto_symbols() -> Vec<SymbolEntry> {
    CRYPTO
        .iter()
        .map(|(t, n)| {
            SymbolEntry::Binance(BinanceSymbol {
                ticker: (*t).to_string(),
                name: (*n).to_string(),
                source: Source::Binance,
                group: SymbolGroup::Crypto,
            })
        })
        .collect()
}

/// Uppercase, strip an `EXCHANGE:` prefix, map gold to the PAXG token, and any other `XXXUSD` to
/// `XXXUSDT` (`hub/server.py:53-57`).
pub fn bsym_of(symbol: &str) -> String {
    let up = symbol.to_uppercase();
    let s = up.rsplit(':').next().unwrap_or("").to_string();
    if s == "XAUUSD" {
        return "PAXGUSDT".to_string();
    }
    if s.ends_with("USD") {
        format!("{s}T")
    } else {
        s
    }
}

pub(crate) fn interval(tf: Tf) -> &'static str {
    match tf {
        Tf::M1 => "1m",
        Tf::M5 => "5m",
        Tf::M15 => "15m",
        Tf::M30 => "30m",
        Tf::H1 => "1h",
        Tf::H4 => "4h",
        Tf::D1 => "1d",
    }
}

fn num(s: &str) -> Result<f64, FeedError> {
    s.parse::<f64>()
        .map_err(|_| FeedError::new(format!("could not convert string to float: '{s}'")))
}

/// Up to 1,000 klines ending at `to` (no pagination, as in Python). `q.symbol` is already mapped.
pub async fn history(
    http: &reqwest::Client,
    base: &str,
    q: &BarsQuery,
) -> Result<Vec<Bar>, FeedError> {
    let mut url = format!(
        "{base}/api/v3/klines?symbol={}&interval={}&limit={}",
        q.symbol,
        interval(q.tf),
        q.limit.min(1000)
    );
    if let Some(to) = q.to_ms {
        url.push_str(&format!("&endTime={to}"));
    }
    let rows: Vec<Vec<serde_json::Value>> = get_json(http, &url).await?;
    rows.iter()
        .map(|k| {
            let cell = |i: usize| {
                k.get(i)
                    .ok_or_else(|| FeedError::new("list index out of range"))
            };
            let text = |i: usize| -> Result<f64, FeedError> {
                match cell(i)? {
                    serde_json::Value::String(s) => num(s),
                    serde_json::Value::Number(n) => {
                        n.as_f64().ok_or_else(|| FeedError::new("bad number"))
                    }
                    _ => Err(FeedError::new("bad kline")),
                }
            };
            let open_ms = cell(0)?
                .as_i64()
                .ok_or_else(|| FeedError::new("bad kline time"))?;
            Ok(Bar {
                time: open_ms / 1000,
                open: text(1)?,
                high: text(2)?,
                low: text(3)?,
                close: text(4)?,
                volume: text(5)?,
            })
        })
        .collect()
}

// ---------------------------------------------------------------- live half (01-02)

use super::{Endpoints, KeepAlive, LivePlan, Parsed, UpstreamKey};
use crate::market::candles::{FeedRules, LatePolicy, Tick, TickBatch};

pub const RULES: FeedRules = FeedRules {
    late: LatePolicy::Merge,
    min_interval: Some(std::time::Duration::from_millis(50)),
    trade_ms_on_wire: true,
};

pub fn live_plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan {
    LivePlan {
        url: format!(
            "{}/ws/{}@aggTrade",
            ep.binance_ws.trim_end_matches('/'),
            key.symbol.as_str().to_lowercase()
        ),
        hello: Vec::new(),
        keepalive: KeepAlive::Protocol,
        rules: RULES,
        end_session_wait: std::time::Duration::from_secs(60),
    }
}

#[derive(serde::Deserialize)]
struct AggTrade {
    p: String,
    q: String,
    #[serde(rename = "T")]
    t: i64,
}

/// One `aggTrade` frame is one trade.
pub fn parse_frame(text: &str) -> Parsed {
    let Ok(a) = serde_json::from_str::<AggTrade>(text) else {
        return Parsed::Ignore;
    };
    match (a.p.parse::<f64>(), a.q.parse::<f64>()) {
        (Ok(price), Ok(qty)) => Parsed::Batch(TickBatch {
            ticks: vec![Tick {
                price,
                qty,
                ts_ms: a.t,
            }],
            src_ms: a.t,
        }),
        _ => Parsed::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::bsym_of;

    #[test]
    fn symbol_map() {
        assert_eq!(bsym_of("XAUUSD"), "PAXGUSDT");
        assert_eq!(bsym_of("btcusd"), "BTCUSDT");
        assert_eq!(bsym_of("BINANCE:ETHUSD"), "ETHUSDT");
        assert_eq!(bsym_of("BTCUSDT"), "BTCUSDT");
        assert_eq!(bsym_of("PAXGUSD"), "PAXGUSDT");
    }
}
