//! Bybit v5: instruments, tickers and klines over REST (`hub/server.py:368-451`). The live
//! `publicTrade` / `tickers` half is 01-02.

use super::get_json;
use crate::market::{BarsQuery, FeedError};
use hub_types::market::{Bar, BybitSymbol, Source, SymbolEntry, SymbolGroup, Tf};
use serde::Deserialize;
use std::collections::HashMap;

const METALS: [&str; 5] = ["XAU", "XAG", "XPT", "XPD", "PAXG"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Linear,
    Spot,
    Inverse,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Linear => "linear",
            Category::Spot => "spot",
            Category::Inverse => "inverse",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Trade,
    Mark,
    Index,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Trade => "trade",
            Kind::Mark => "mark",
            Kind::Index => "index",
        }
    }
}

/// A parsed hub ticker: `BYBIT_S_BTCUSDT.MARK` is spot, `BTCUSDT`, mark (`bybit_parse`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BybitTicker {
    pub category: Category,
    pub symbol: String,
    pub kind: Kind,
}

impl BybitTicker {
    /// Python strips the first six characters (`BYBIT_`) without checking them.
    pub fn parse(ticker: &str) -> BybitTicker {
        let mut t = ticker.get(6..).unwrap_or("");
        let mut kind = Kind::Trade;
        if let Some(rest) = t.strip_suffix(".MARK") {
            t = rest;
            kind = Kind::Mark;
        } else if let Some(rest) = t.strip_suffix(".INDEX") {
            t = rest;
            kind = Kind::Index;
        }
        let mut category = Category::Linear;
        if let Some(rest) = t.strip_prefix("S_") {
            t = rest;
            category = Category::Spot;
        } else if let Some(rest) = t.strip_prefix("I_") {
            t = rest;
            category = Category::Inverse;
        }
        BybitTicker {
            category,
            symbol: t.to_string(),
            kind,
        }
    }
}

pub(crate) fn interval(tf: Tf) -> &'static str {
    match tf {
        Tf::M1 => "1",
        Tf::M5 => "5",
        Tf::M15 => "15",
        Tf::M30 => "30",
        Tf::H1 => "60",
        Tf::H4 => "240",
        Tf::D1 => "D",
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<T> {
    ret_code: i64,
    #[serde(default)]
    ret_msg: String,
    result: Option<T>,
}

/// `bybit_get`: a non-zero `retCode` is an error with Bybit's message.
async fn bybit_get<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    base: &str,
    path: &str,
) -> Result<T, FeedError> {
    let env: Envelope<T> = get_json(http, &format!("{base}{path}")).await?;
    if env.ret_code != 0 {
        return Err(FeedError::new(format!("Bybit: {}", env.ret_msg)));
    }
    env.result
        .ok_or_else(|| FeedError::new("Bybit: empty result"))
}

#[derive(Deserialize)]
struct KlineList {
    list: Vec<Vec<String>>,
}

fn num(s: &str) -> Result<f64, FeedError> {
    s.parse::<f64>()
        .map_err(|_| FeedError::new(format!("could not convert string to float: '{s}'")))
}

/// Up to 1,000 candles (no pagination). Bybit returns newest first; mark and index rows have no
/// volume (0.0).
pub async fn history(
    http: &reqwest::Client,
    base: &str,
    q: &BarsQuery,
) -> Result<Vec<Bar>, FeedError> {
    let t = BybitTicker::parse(q.symbol.as_str());
    let ep = match t.kind {
        Kind::Trade => "kline",
        Kind::Mark => "mark-price-kline",
        Kind::Index => "index-price-kline",
    };
    let mut path = format!(
        "/v5/market/{ep}?category={}&symbol={}&interval={}&limit={}",
        t.category.as_str(),
        t.symbol,
        interval(q.tf),
        q.limit.min(1000)
    );
    if let Some(to) = q.to_ms {
        path.push_str(&format!("&end={to}"));
    }
    let rows: KlineList = bybit_get(http, base, &path).await?;
    rows.list
        .iter()
        .rev()
        .map(|k| {
            let field = |i: usize| {
                k.get(i)
                    .map(String::as_str)
                    .ok_or_else(|| FeedError::new("list index out of range"))
            };
            let open_ms: i64 = field(0)?
                .parse()
                .map_err(|_| FeedError::new("bad kline time"))?;
            Ok(Bar {
                time: open_ms / 1000,
                open: num(field(1)?)?,
                high: num(field(2)?)?,
                low: num(field(3)?)?,
                close: num(field(4)?)?,
                volume: if k.len() > 5 { num(&k[5])? } else { 0.0 },
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstrumentPage {
    list: Vec<InstrumentRow>,
    #[serde(default)]
    next_page_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstrumentRow {
    symbol: String,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    base_coin: Option<String>,
    #[serde(default)]
    quote_coin: Option<String>,
    #[serde(default)]
    contract_type: Option<String>,
    #[serde(default)]
    price_filter: Option<PriceFilter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PriceFilter {
    #[serde(default)]
    tick_size: Option<String>,
}

#[derive(Deserialize)]
struct TickerList {
    list: Vec<TickerRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TickerRow {
    symbol: String,
    #[serde(default)]
    turnover24h: Option<String>,
}

/// Decimal places of a tick size string, as Python counts them (`"0.10"` is 1, `"1"` is 0).
fn digits_of(tick: &str) -> u32 {
    let t = tick.trim_end_matches('0');
    match t.split_once('.') {
        Some((_, frac)) => frac.len() as u32,
        None => 0,
    }
}

fn group_order(g: SymbolGroup) -> usize {
    match g {
        SymbolGroup::BybitMetal => 0,
        SymbolGroup::BybitPerp => 1,
        SymbolGroup::BybitSpot => 2,
        SymbolGroup::BybitFutures => 3,
        SymbolGroup::BybitInverse => 4,
        SymbolGroup::BybitMark => 5,
        SymbolGroup::BybitIndex => 6,
        _ => 7,
    }
}

/// Every trading Bybit market with its mark and index variants (non-spot), sorted by group, then
/// by 24 h turnover, highest first (`bybit_load_symbols`).
pub async fn symbols(http: &reqwest::Client, base: &str) -> Result<Vec<SymbolEntry>, FeedError> {
    let mut out: Vec<(f64, BybitSymbol)> = Vec::new();
    for (cat, prefix) in [
        (Category::Linear, ""),
        (Category::Spot, "S_"),
        (Category::Inverse, "I_"),
    ] {
        let mut rows: Vec<InstrumentRow> = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut path = format!(
                "/v5/market/instruments-info?category={}&limit=1000",
                cat.as_str()
            );
            if !cursor.is_empty() {
                path.push_str(&format!("&cursor={cursor}"));
            }
            let page: InstrumentPage = bybit_get(http, base, &path).await?;
            rows.extend(page.list);
            match page.next_page_cursor.filter(|c| !c.is_empty()) {
                Some(c) => cursor = c,
                None => break,
            }
        }
        let tickers: TickerList = bybit_get(
            http,
            base,
            &format!("/v5/market/tickers?category={}", cat.as_str()),
        )
        .await?;
        let turnover: HashMap<String, f64> = tickers
            .list
            .into_iter()
            .map(|t| {
                (
                    t.symbol,
                    t.turnover24h.and_then(|v| v.parse().ok()).unwrap_or(0.0),
                )
            })
            .collect();
        for s in rows {
            if s.status.as_deref() != Some("Trading") {
                continue;
            }
            let tick = s
                .price_filter
                .and_then(|p| p.tick_size)
                .unwrap_or_else(|| "0.01".to_string());
            let digits = digits_of(&tick);
            let base_coin = s.base_coin.unwrap_or_default();
            let ctype = s.contract_type.unwrap_or_default();
            let upper = base_coin.to_uppercase();
            let group = if METALS.iter().any(|m| upper.starts_with(m)) || upper == "XAUT" {
                SymbolGroup::BybitMetal
            } else if cat == Category::Spot {
                SymbolGroup::BybitSpot
            } else if cat == Category::Inverse {
                SymbolGroup::BybitInverse
            } else if ctype == "LinearFutures" {
                SymbolGroup::BybitFutures
            } else {
                SymbolGroup::BybitPerp
            };
            let label = match cat {
                Category::Linear if ctype != "LinearFutures" => "Perp",
                Category::Linear => "Futures",
                Category::Spot => "Spot",
                Category::Inverse => "Inverse",
            };
            let t = turnover.get(&s.symbol).copied().unwrap_or(0.0);
            let row = BybitSymbol {
                ticker: format!("BYBIT_{prefix}{}", s.symbol),
                name: format!(
                    "Bybit {label} - {base_coin}/{}",
                    s.quote_coin.unwrap_or_default()
                ),
                source: Source::Bybit,
                group,
                digits,
            };
            let variants: Vec<BybitSymbol> = if cat != Category::Spot {
                [
                    ("MARK", SymbolGroup::BybitMark),
                    ("INDEX", SymbolGroup::BybitIndex),
                ]
                .into_iter()
                .map(|(kind, g)| BybitSymbol {
                    ticker: format!("{}.{kind}", row.ticker),
                    name: format!("{} ({} price)", row.name, kind.to_lowercase()),
                    group: g,
                    ..row.clone()
                })
                .collect()
            } else {
                Vec::new()
            };
            out.push((t, row));
            out.extend(variants.into_iter().map(|v| (t, v)));
        }
    }
    out.sort_by(|a, b| {
        group_order(a.1.group)
            .cmp(&group_order(b.1.group))
            .then_with(|| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal))
    });
    Ok(out
        .into_iter()
        .map(|(_, s)| SymbolEntry::Bybit(s))
        .collect())
}

// ---------------------------------------------------------------- live half (01-02)

use super::{Endpoints, KeepAlive, LivePlan, Parsed, UpstreamKey};
use crate::market::candles::{FeedRules, LatePolicy, Tick, TickBatch};

pub const RULES: FeedRules = FeedRules {
    late: LatePolicy::Drop,
    min_interval: Some(std::time::Duration::from_millis(50)),
    trade_ms_on_wire: true,
};

fn topic_of(t: &BybitTicker) -> String {
    match t.kind {
        Kind::Trade => format!("publicTrade.{}", t.symbol),
        Kind::Mark | Kind::Index => format!("tickers.{}", t.symbol),
    }
}

pub fn live_plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan {
    let t = BybitTicker::parse(key.symbol.as_str());
    LivePlan {
        url: format!("{}{}", ep.bybit_ws, t.category.as_str()),
        hello: vec![serde_json::json!({"op": "subscribe", "args": [topic_of(&t)]}).to_string()],
        keepalive: KeepAlive::JsonPing {
            every: std::time::Duration::from_secs(20),
        },
        rules: RULES,
        end_session_wait: std::time::Duration::from_secs(2),
    }
}

fn json_num(x: &serde_json::Value) -> Option<f64> {
    match x {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// `publicTrade` frames carry a batch of trades; `tickers` frames carry a mark or index price
/// (delta frames without the field are ignored). Other topics, acks and pongs are ignored.
pub fn parse_frame(ticker: &str, text: &str) -> Parsed {
    let t = BybitTicker::parse(ticker);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Parsed::Ignore;
    };
    if v.get("topic").and_then(|x| x.as_str()) != Some(topic_of(&t).as_str()) {
        return Parsed::Ignore;
    }
    match t.kind {
        Kind::Trade => {
            let Some(rows) = v.get("data").and_then(|d| d.as_array()) else {
                return Parsed::Ignore;
            };
            let ticks: Vec<Tick> = rows
                .iter()
                .filter_map(|r| {
                    Some(Tick {
                        price: json_num(&r["p"])?,
                        qty: json_num(&r["v"])?,
                        ts_ms: r["T"].as_i64()?,
                    })
                })
                .collect();
            match ticks.last() {
                Some(last) => Parsed::Batch(TickBatch {
                    src_ms: last.ts_ms,
                    ticks,
                }),
                None => Parsed::Ignore,
            }
        }
        Kind::Mark | Kind::Index => {
            let field = if t.kind == Kind::Mark {
                "markPrice"
            } else {
                "indexPrice"
            };
            let Some(price) = v.get("data").and_then(|d| d.get(field)).and_then(json_num) else {
                return Parsed::Ignore;
            };
            if price == 0.0 {
                return Parsed::Ignore;
            }
            let Some(ts) = v.get("ts").and_then(|x| x.as_i64()) else {
                return Parsed::Ignore;
            };
            Parsed::Batch(TickBatch {
                ticks: vec![Tick {
                    price,
                    qty: 0.0,
                    ts_ms: ts,
                }],
                src_ms: ts,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticker_parse_table() {
        let cases = [
            ("BYBIT_BTCUSDT", Category::Linear, "BTCUSDT", Kind::Trade),
            ("BYBIT_S_BTCUSDT", Category::Spot, "BTCUSDT", Kind::Trade),
            (
                "BYBIT_I_BTCUSD.INDEX",
                Category::Inverse,
                "BTCUSD",
                Kind::Index,
            ),
            (
                "BYBIT_ETHUSDT.MARK",
                Category::Linear,
                "ETHUSDT",
                Kind::Mark,
            ),
            (
                "BYBIT_BTC-27MAR26",
                Category::Linear,
                "BTC-27MAR26",
                Kind::Trade,
            ),
        ];
        for (raw, cat, sym, kind) in cases {
            assert_eq!(
                BybitTicker::parse(raw),
                BybitTicker {
                    category: cat,
                    symbol: sym.into(),
                    kind
                },
                "{raw}"
            );
        }
    }

    #[test]
    fn digits() {
        assert_eq!(digits_of("0.10"), 1);
        assert_eq!(digits_of("0.0001"), 4);
        assert_eq!(digits_of("1"), 0);
        assert_eq!(digits_of("10"), 0);
        assert_eq!(digits_of("0.5"), 1);
    }
}
