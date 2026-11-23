//! Deriv: symbols and candles over its public websocket API (`hub/server.py:553-649`). The live
//! `ticks` half is 01-02.
//!
//! Opening a Deriv connection costs about 1.5 s, more than a request, and Deriv returns at most
//! 1,000 candles per reply, so requests reuse a small pool of open sockets and a deep chart asks
//! for 1,000-candle windows in parallel.

use crate::market::{BarsQuery, FeedError};
use futures_util::{stream, SinkExt, StreamExt};
use hub_types::market::{Bar, DerivSymbol, Source, SymbolEntry, SymbolGroup};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub const DERIV_URL: &str = "wss://api.derivws.com/trading/v1/options/ws/public";
const POOL_MAX: usize = 6;
const WINDOW: i64 = 1000;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A pool of open request sockets with `req_id` matching (`deriv_request`).
pub struct RpcPool {
    url: String,
    idle: Mutex<Vec<Ws>>,
    next_id: AtomicU64,
}

impl RpcPool {
    pub fn new(url: String) -> RpcPool {
        RpcPool {
            url,
            idle: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// Sends one request and returns the reply whose `req_id` matches (or has none). Retries once
    /// on a fresh socket; a socket that answered is returned to the pool (at most six kept).
    pub async fn request(&self, mut req: Value) -> Result<Value, FeedError> {
        let rid = self.next_id.fetch_add(1, Ordering::Relaxed);
        req["req_id"] = json!(rid);
        let text = req.to_string();
        let mut pooled = self.idle.lock().map(|mut v| v.pop()).unwrap_or(None);
        let mut last_err = FeedError::new("Deriv request failed");
        for _attempt in 0..2 {
            let mut ws = match pooled.take() {
                Some(ws) => ws,
                None => match tokio::time::timeout(
                    Duration::from_secs(10),
                    tokio_tungstenite::connect_async(&self.url),
                )
                .await
                {
                    Ok(Ok((ws, _))) => ws,
                    Ok(Err(e)) => {
                        last_err = FeedError::new(e.to_string());
                        continue;
                    }
                    Err(_) => {
                        last_err = FeedError::new("timed out during opening handshake");
                        continue;
                    }
                },
            };
            match Self::round_trip(&mut ws, &text, rid).await {
                Ok(reply) => {
                    if let Ok(mut idle) = self.idle.lock() {
                        if idle.len() < POOL_MAX {
                            idle.push(ws);
                        }
                    }
                    return Ok(reply);
                }
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    async fn round_trip(ws: &mut Ws, text: &str, rid: u64) -> Result<Value, FeedError> {
        ws.send(Message::Text(text.to_string().into()))
            .await
            .map_err(|e| FeedError::new(e.to_string()))?;
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(20), ws.next())
                .await
                .map_err(|_| FeedError::new("timed out"))?
                .ok_or_else(|| FeedError::new("connection closed"))?
                .map_err(|e| FeedError::new(e.to_string()))?;
            let Message::Text(t) = msg else { continue };
            let r: Value = serde_json::from_str(&t).map_err(|e| FeedError::new(e.to_string()))?;
            match r.get("req_id") {
                None | Some(Value::Null) => return Ok(r),
                Some(v) if v.as_u64() == Some(rid) => return Ok(r),
                Some(_) => continue,
            }
        }
    }
}

fn granularity(q: &BarsQuery) -> i64 {
    q.tf.secs()
}

fn as_f64(v: &Value) -> Result<f64, FeedError> {
    match v {
        Value::Number(n) => n.as_f64().ok_or_else(|| FeedError::new("bad number")),
        Value::String(s) => s
            .parse()
            .map_err(|_| FeedError::new(format!("could not convert string to float: '{s}'"))),
        _ => Err(FeedError::new("bad number")),
    }
}

/// `_deriv_bars`: an `error` reply raises with Deriv's message; volume is always 0.0.
fn bars_of(r: &Value) -> Result<Vec<Bar>, FeedError> {
    if let Some(err) = r.get("error").filter(|e| !e.is_null()) {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Deriv error");
        return Err(FeedError::new(msg));
    }
    let Some(candles) = r.get("candles").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    candles
        .iter()
        .map(|c| {
            Ok(Bar {
                time: as_f64(&c["epoch"])? as i64,
                open: as_f64(&c["open"])?,
                high: as_f64(&c["high"])?,
                low: as_f64(&c["low"])?,
                close: as_f64(&c["close"])?,
                volume: 0.0,
            })
        })
        .collect()
}

/// Back-to-back windows of 1,000 candles ending at `end` (aligned to the granularity), newest
/// first: `(start, end)` pairs as Python builds them. Pure.
pub fn window_plan(end: i64, g: i64, n: usize) -> Vec<(i64, i64)> {
    let span = WINDOW * g;
    (0..n as i64)
        .map(|k| (end - (k + 1) * span + g, end - k * span))
        .collect()
}

/// How many windows to ask for next: `max(1, min(10, ceil((limit - got) / 1000)))`.
pub fn windows_needed(limit: i64, got: usize) -> usize {
    let missing = (limit - got as i64).max(0);
    let n = (missing + WINDOW - 1) / WINDOW;
    n.clamp(1, 10) as usize
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Candles for one query. Up to 1,000: one request. More: windows in parallel, five at a time,
/// stepping back until enough candles or four empty batches (closed-market weekends).
pub async fn history(pool: &RpcPool, q: &BarsQuery) -> Result<Vec<Bar>, FeedError> {
    let g = granularity(q);
    if q.limit <= WINDOW {
        let end = match q.to_ms {
            Some(to) => json!((to as f64 / 1000.0) as i64).to_string(),
            None => "latest".to_string(),
        };
        let r = pool
            .request(
                json!({"ticks_history": q.symbol.as_str(), "style": "candles", "granularity": g,
                            "count": q.limit, "end": end, "adjust_start_time": 1}),
            )
            .await?;
        return bars_of(&r);
    }
    let mut end = q
        .to_ms
        .map(|to| (to as f64 / 1000.0) as i64)
        .unwrap_or_else(now_s);
    end -= end.rem_euclid(g);
    let mut got: BTreeMap<i64, Bar> = BTreeMap::new();
    let mut empty = 0;
    while (got.len() as i64) < q.limit && empty < 4 {
        let n = windows_needed(q.limit, got.len());
        let reqs: Vec<Value> = window_plan(end, g, n)
            .into_iter()
            .map(|(a, b)| {
                json!({"ticks_history": q.symbol.as_str(), "style": "candles", "granularity": g,
                                 "start": a, "end": b, "count": WINDOW})
            })
            .collect();
        let before = got.len();
        let results: Vec<Result<Vec<Bar>, FeedError>> = stream::iter(reqs)
            .map(|r| async move { bars_of(&pool.request(r).await?) })
            .buffered(5)
            .collect()
            .await;
        for res in results {
            for bar in res? {
                got.insert(bar.time, bar);
            }
        }
        empty = if got.len() == before { empty + 1 } else { 0 };
        end -= n as i64 * WINDOW * g;
    }
    let all: Vec<Bar> = got.into_values().collect();
    let skip = all.len().saturating_sub(q.limit.max(0) as usize);
    Ok(all.into_iter().skip(skip).collect())
}

#[derive(Deserialize)]
struct ActiveSymbol {
    underlying_symbol: String,
    #[serde(default)]
    underlying_symbol_name: Option<String>,
    #[serde(default)]
    market: Option<String>,
    #[serde(default)]
    pip_size: Option<f64>,
    #[serde(default)]
    exchange_is_open: Option<Value>,
}

fn group_of(market: &str) -> Option<SymbolGroup> {
    match market {
        "forex" => Some(SymbolGroup::Forex),
        "commodities" => Some(SymbolGroup::Commodity),
        "cryptocurrency" => Some(SymbolGroup::Crypto),
        "indices" => Some(SymbolGroup::Indices),
        _ => None,
    }
}

fn group_order(g: SymbolGroup) -> usize {
    match g {
        SymbolGroup::Commodity => 0,
        SymbolGroup::Forex => 1,
        SymbolGroup::Indices => 2,
        SymbolGroup::Crypto => 3,
        _ => 4,
    }
}

/// `round(-log10(pip))` for a pip under 1, else 0, never below 0.
fn digits_of(pip: f64) -> u32 {
    if pip < 1.0 && pip > 0.0 {
        (-pip.log10()).round().max(0.0) as u32
    } else {
        0
    }
}

fn truthy(v: &Option<Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        _ => false,
    }
}

/// The Deriv symbol list (`deriv_load_symbols`): four markets, sorted by group then ticker.
pub async fn symbols(pool: &RpcPool) -> Result<Vec<SymbolEntry>, FeedError> {
    let r = pool.request(json!({"active_symbols": "brief"})).await?;
    if let Some(err) = r.get("error").filter(|e| !e.is_null()) {
        return Err(FeedError::new(
            err.get("message")
                .and_then(Value::as_str)
                .unwrap_or("Deriv error"),
        ));
    }
    let rows: Vec<ActiveSymbol> = serde_json::from_value(
        r.get("active_symbols")
            .cloned()
            .unwrap_or(Value::Array(vec![])),
    )
    .map_err(|e| FeedError::new(format!("bad active_symbols: {e}")))?;
    let mut out: Vec<DerivSymbol> = rows
        .into_iter()
        .filter_map(|s| {
            let group = group_of(s.market.as_deref().unwrap_or(""))?;
            let pip = s.pip_size.filter(|p| *p != 0.0).unwrap_or(0.01);
            Some(DerivSymbol {
                name: format!(
                    "{} (Deriv)",
                    s.underlying_symbol_name
                        .clone()
                        .unwrap_or_else(|| s.underlying_symbol.clone())
                ),
                ticker: s.underlying_symbol,
                source: Source::Deriv,
                group,
                digits: digits_of(pip),
                open: truthy(&s.exchange_is_open),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        group_order(a.group)
            .cmp(&group_order(b.group))
            .then_with(|| a.ticker.cmp(&b.ticker))
    });
    Ok(out.into_iter().map(SymbolEntry::Deriv).collect())
}

// ---------------------------------------------------------------- live half (01-02)

use super::{Endpoints, KeepAlive, LivePlan, Parsed, UpstreamKey};
use crate::market::candles::{FeedRules, LatePolicy, Tick, TickBatch};

pub const RULES: FeedRules = FeedRules {
    late: LatePolicy::Merge,
    min_interval: None,
    trade_ms_on_wire: false,
};

pub fn live_plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan {
    LivePlan {
        url: ep.deriv_ws.clone(),
        hello: vec![json!({"ticks": key.symbol.as_str(), "subscribe": 1}).to_string()],
        keepalive: KeepAlive::Protocol,
        rules: RULES,
        // Python waited 60 s after an error reply (market closed) before trying again.
        end_session_wait: Duration::from_secs(60),
    }
}

/// A `tick` frame is one tick (whole-second epoch, no volume); an `error` reply ends the session.
pub fn parse_frame(text: &str) -> Parsed {
    let Ok(m) = serde_json::from_str::<Value>(text) else {
        return Parsed::Ignore;
    };
    if let Some(err) = m.get("error").filter(|e| !e.is_null()) {
        let msg = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("error");
        return Parsed::EndSession {
            status: format!("Deriv: {msg}"),
        };
    }
    let Some(t) = m.get("tick").filter(|t| t.is_object()) else {
        return Parsed::Ignore;
    };
    let (Ok(price), Ok(epoch)) = (as_f64(&t["quote"]), as_f64(&t["epoch"])) else {
        return Parsed::Ignore;
    };
    let ts_ms = epoch as i64 * 1000;
    Parsed::Batch(TickBatch {
        ticks: vec![Tick {
            price,
            qty: 0.0,
            ts_ms,
        }],
        src_ms: ts_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_match_python() {
        // end aligned to 60 s; three windows of 1,000 minutes, newest first.
        let w = window_plan(1_000_020, 60, 3);
        assert_eq!(
            w,
            vec![(940_080, 1_000_020), (880_080, 940_020), (820_080, 880_020)]
        );
        assert_eq!(windows_needed(10_000, 0), 10);
        assert_eq!(windows_needed(25_000, 0), 10);
        assert_eq!(windows_needed(1_500, 1_200), 1);
        assert_eq!(windows_needed(1_000, 2_000), 1);
    }

    #[test]
    fn digits_and_bars() {
        assert_eq!(digits_of(0.001), 3);
        assert_eq!(digits_of(0.01), 2);
        assert_eq!(digits_of(1.0), 0);
        let r =
            json!({"candles": [{"epoch": 60, "open": 1.5, "high": 2, "low": "1", "close": 1.75}]});
        assert_eq!(
            bars_of(&r).unwrap()[0],
            Bar {
                time: 60,
                open: 1.5,
                high: 2.0,
                low: 1.0,
                close: 1.75,
                volume: 0.0
            }
        );
        assert_eq!(
            bars_of(&json!({"error": {"message": "Market is closed."}}))
                .unwrap_err()
                .0,
            "Market is closed."
        );
    }
}
