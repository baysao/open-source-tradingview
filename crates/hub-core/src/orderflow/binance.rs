//! Binance USD-M futures: wire formats into domain types (02-01). The transport, the shared weight
//! budget and the futures list are added in 02-02. Wire DTOs stay private to this file.

use super::calc::{py_float, Kline, TradeCols};
use super::OfError;
use serde_json::Value;
use std::collections::BTreeSet;

fn bad(what: &str) -> OfError {
    OfError::value(format!("unexpected Binance {what} reply"))
}

/// A JSON value Python would pass to `float()`: Binance sends decimals as strings.
fn num(v: &Value, what: &str) -> Result<f64, OfError> {
    match v {
        Value::String(s) => py_float(s),
        Value::Number(n) => n.as_f64().ok_or_else(|| bad(what)),
        _ => Err(bad(what)),
    }
}

fn int(v: &Value, what: &str) -> Result<i64, OfError> {
    v.as_i64().ok_or_else(|| bad(what))
}

/// `/fapi/v1/klines`: `[[open_ms, "o", "h", "l", "c", "v", close_ms, "qv", n, "taker_buy", ...], ...]`.
pub fn parse_klines(body: &[u8]) -> Result<Vec<Kline>, OfError> {
    let rows: Vec<Vec<Value>> = serde_json::from_slice(body).map_err(|_| bad("klines"))?;
    rows.iter()
        .map(|k| {
            if k.len() < 10 {
                return Err(bad("klines"));
            }
            Ok(Kline {
                open_ms: int(&k[0], "klines")?,
                high: num(&k[2], "klines")?,
                low: num(&k[3], "klines")?,
                close: num(&k[4], "klines")?,
                volume: num(&k[5], "klines")?,
                taker_buy: num(&k[9], "klines")?,
            })
        })
        .collect()
}

/// `/fapi/v1/exchangeInfo`: the symbols whose status is `TRADING`.
pub fn parse_exchange_info(body: &[u8]) -> Result<BTreeSet<String>, OfError> {
    let v: Value = serde_json::from_slice(body).map_err(|_| bad("exchangeInfo"))?;
    let syms = v["symbols"].as_array().ok_or_else(|| bad("exchangeInfo"))?;
    Ok(syms
        .iter()
        .filter(|s| s["status"].as_str() == Some("TRADING"))
        .filter_map(|s| s["symbol"].as_str().map(str::to_string))
        .collect())
}

/// `/fapi/v1/aggTrades`: `[{"a": id, "p": "...", "q": "...", "T": ms, "m": bool}, ...]` into ids
/// and columns (side -1 when the buyer was the maker, so the seller crossed the spread).
pub fn parse_agg_trades(body: &[u8]) -> Result<(Vec<i64>, TradeCols), OfError> {
    let rows: Vec<Value> = serde_json::from_slice(body).map_err(|_| bad("aggTrades"))?;
    let mut ids = Vec::with_capacity(rows.len());
    let mut t = TradeCols::default();
    for r in &rows {
        ids.push(int(&r["a"], "aggTrades")?);
        t.t_ms.push(int(&r["T"], "aggTrades")?);
        t.price.push(num(&r["p"], "aggTrades")?);
        t.qty.push(num(&r["q"], "aggTrades")?);
        t.side
            .push(if r["m"].as_bool().ok_or_else(|| bad("aggTrades"))? {
                -1
            } else {
                1
            });
    }
    Ok((ids, t))
}

// ---------------------------------------------------------------- 02-02: budget, transport, futures list

use super::calc::py_int;
use super::PyClass;
use futures_util::future::BoxFuture;
use hub_types::market::Tf;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

/// Binance allows 2,400 weight per minute per IP; this server uses at most `BUDGET`.
pub const BUDGET: u32 = 1900;
/// Background loading leaves this much free so a chart request never waits behind it.
pub const RESERVE: u32 = 400;

/// Who is asking: a chart request, or background loading (back-fill, day files).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prio {
    Chart,
    Background,
}

/// `_Weight`'s state. One per process (owned by `Fapi`), written by every caller under a short
/// std lock that is never held across a sleep or a request.
#[derive(Debug, Default)]
pub struct WeightState {
    used: VecDeque<(Instant, u32)>,
    until: Option<Instant>,
    server_used: u32,
}

impl WeightState {
    /// One step of `_Weight.take`: `Ok` takes `w` now; `Err(sleep)` asks the caller to wait and
    /// call again. Clears `server_used` when the reply header was what blocked (it is re-read on
    /// the next reply), as Python does.
    pub fn decide(&mut self, now: Instant, w: u32, p: Prio) -> Result<(), Duration> {
        let low = p == Prio::Background;
        let cap = BUDGET - if low { RESERVE } else { 0 };
        let header_cap = if low { 1800 } else { 2200 };
        while self
            .used
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) >= Duration::from_secs(60))
        {
            self.used.pop_front();
        }
        let spent: u32 = self.used.iter().fold(0, |a, (_, x)| a + x);
        let banned = self.until.filter(|u| *u > now);
        if banned.is_none() && spent + w <= cap && self.server_used + w < header_cap {
            self.used.push_back((now, w));
            return Ok(());
        }
        let wait = match banned {
            Some(u) => u - now,
            None => {
                let wait = match self.used.front() {
                    Some((t, _)) => Duration::from_secs(60).saturating_sub(now.duration_since(*t)),
                    None => Duration::from_secs(1),
                };
                if self.server_used + w >= header_cap {
                    self.server_used = 0;
                }
                wait
            }
        };
        Err(wait.clamp(Duration::from_millis(200), Duration::from_secs(5)))
    }

    /// After a reply: `X-MBX-USED-WEIGHT-1M`, and on 418 or 429 no calls for `Retry-After`
    /// seconds (60 when absent).
    pub fn note_reply(
        &mut self,
        now: Instant,
        status: u16,
        used_1m: Option<u32>,
        retry_after: Option<u64>,
    ) {
        if let Some(u) = used_1m {
            self.server_used = u;
        }
        if status == 418 || status == 429 {
            self.until = Some(now + Duration::from_secs(retry_after.unwrap_or(60)));
        }
    }
}

/// Binance USD-M futures and the public data archive over one HTTP client and one weight budget.
#[derive(Clone)]
pub struct Fapi {
    http: reqwest::Client,
    fapi: String,
    vision: String,
    weight: Arc<Mutex<WeightState>>,
}

fn binance_tf(tf: Tf) -> &'static str {
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

/// An upstream failure as Python's urllib reports it.
fn http_error(status: reqwest::StatusCode) -> OfError {
    OfError::new(
        PyClass::HTTPError,
        format!(
            "HTTP Error {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        ),
    )
}

fn send_error(e: reqwest::Error) -> OfError {
    if e.is_timeout() {
        OfError::new(PyClass::TimeoutError, "timed out")
    } else {
        OfError::new(PyClass::URLError, format!("<urlopen error {e}>"))
    }
}

impl Fapi {
    /// `fapi` and `vision` are base URLs (`Endpoints.binance_fapi`, `binance_vision`).
    pub fn new(fapi: &str, vision: &str) -> Fapi {
        let http = reqwest::Client::builder()
            .user_agent("tradex/1.0")
            .build()
            .expect("reqwest client builds with static settings");
        Fapi {
            http,
            fapi: fapi.trim_end_matches('/').to_string(),
            vision: vision.trim_end_matches('/').to_string(),
            weight: Arc::default(),
        }
    }

    /// Waits until the budget allows `w`, then takes it.
    pub async fn take(&self, w: u32, p: Prio) {
        loop {
            let step = self
                .weight
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .decide(Instant::now(), w, p);
            match step {
                Ok(()) => return,
                Err(sleep) => tokio::time::sleep(sleep).await,
            }
        }
    }

    /// A weighted GET (`WEIGHT.get`): budget, request, header, ban on 418/429.
    async fn weighted(&self, url: &str, w: u32, p: Prio) -> Result<Vec<u8>, OfError> {
        self.take(w, p).await;
        let resp = self
            .http
            .get(url)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(send_error)?;
        let status = resp.status();
        let header = |k: &str| {
            resp.headers()
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let used = header("X-MBX-USED-WEIGHT-1M")
            .and_then(|v| py_int(&v).ok())
            .and_then(|v| u32::try_from(v).ok());
        let retry = header("Retry-After").and_then(|v| v.parse::<u64>().ok());
        self.weight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .note_reply(Instant::now(), status.as_u16(), used, retry);
        if !status.is_success() {
            return Err(http_error(status));
        }
        resp.bytes().await.map(|b| b.to_vec()).map_err(send_error)
    }

    /// One klines page (`_binance_rows`): weight 10, chart priority, up to 1,500 rows.
    pub async fn klines(
        &self,
        sym: &str,
        itf: Tf,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<Vec<Kline>, OfError> {
        let url = format!(
            "{}/fapi/v1/klines?symbol={sym}&interval={}&startTime={start_ms}&endTime={end_ms}&limit=1500",
            self.fapi,
            binance_tf(itf)
        );
        parse_klines(&self.weighted(&url, 10, Prio::Chart).await?)
    }

    /// One aggTrades page (`TradeStore._rest`): weight 20, 1,000 trades, the newest page without
    /// `from_id`. `None` for an empty page.
    pub async fn agg_trades(
        &self,
        sym: &str,
        from_id: Option<i64>,
        p: Prio,
    ) -> Result<Option<(Vec<i64>, TradeCols)>, OfError> {
        let mut url = format!("{}/fapi/v1/aggTrades?symbol={sym}&limit=1000", self.fapi);
        if let Some(id) = from_id {
            url.push_str(&format!("&fromId={id}"));
        }
        let (ids, t) = parse_agg_trades(&self.weighted(&url, 20, p).await?)?;
        Ok((!ids.is_empty()).then_some((ids, t)))
    }

    /// The TRADING USD-M symbols (`futures_symbols`): not weighted, 20 s, as Python.
    pub async fn exchange_info(&self) -> Result<BTreeSet<String>, OfError> {
        let url = format!("{}/fapi/v1/exchangeInfo", self.fapi);
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(send_error)?;
        if !resp.status().is_success() {
            return Err(http_error(resp.status()));
        }
        parse_exchange_info(&resp.bytes().await.map_err(send_error)?)
    }

    /// One daily aggTrades archive (`fetch_day`): not weighted, 120 s.
    pub async fn daily_zip(&self, sym: &str, day: &str) -> Result<Vec<u8>, OfError> {
        let url = format!(
            "{}/data/futures/um/daily/aggTrades/{sym}/{sym}-aggTrades-{day}.zip",
            self.vision
        );
        let resp = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(send_error)?;
        if !resp.status().is_success() {
            return Err(http_error(resp.status()));
        }
        resp.bytes().await.map(|b| b.to_vec()).map_err(send_error)
    }
}

/// How the futures list is fetched (the real one is `Fapi::exchange_info`; tests inject one).
pub type FetchFutures =
    Arc<dyn Fn() -> BoxFuture<'static, Result<BTreeSet<String>, OfError>> + Send + Sync>;

struct FuturesState {
    set: Arc<BTreeSet<String>>,
    next: Option<Instant>,
}

/// `futures_symbols()`: empty until the first request fetches it (as Python, L-011), refreshed
/// every 6 h; a failure keeps the old set and retries after 5 minutes. One fetch at a time: the
/// tokio mutex is held across the fetch on purpose, so concurrent callers wait for its result.
#[derive(Clone)]
pub struct FuturesList {
    fetch: FetchFutures,
    state: Arc<tokio::sync::Mutex<FuturesState>>,
}

pub const FUTURES_TTL: Duration = Duration::from_secs(6 * 3600);
pub const FUTURES_RETRY: Duration = Duration::from_secs(300);

impl FuturesList {
    pub fn new(fetch: FetchFutures) -> FuturesList {
        FuturesList {
            fetch,
            state: Arc::new(tokio::sync::Mutex::new(FuturesState {
                set: Arc::default(),
                next: None,
            })),
        }
    }

    pub fn from_fapi(fapi: Fapi) -> FuturesList {
        FuturesList::new(Arc::new(move || {
            let f = fapi.clone();
            Box::pin(async move { f.exchange_info().await })
        }))
    }

    pub async fn get(&self) -> Arc<BTreeSet<String>> {
        let mut st = self.state.lock().await;
        let now = Instant::now();
        if st.next.is_none_or(|n| now >= n) {
            match (self.fetch)().await {
                Ok(set) => {
                    st.set = Arc::new(set);
                    st.next = Some(now + FUTURES_TTL);
                }
                Err(_) => st.next = Some(now + FUTURES_RETRY),
            }
        }
        st.set.clone()
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test(start_paused = true)]
    async fn chart_and_background_caps() {
        let t0 = Instant::now();
        let mut w = WeightState::default();
        for _ in 0..95 {
            assert!(w.decide(t0, 20, Prio::Chart).is_ok());
        }
        assert!(w.decide(t0, 20, Prio::Chart).is_err(), "1900 spent");
        let mut b = WeightState::default();
        for _ in 0..75 {
            assert!(b.decide(t0, 20, Prio::Background).is_ok());
        }
        assert!(
            b.decide(t0, 20, Prio::Background).is_err(),
            "background stops at 1500"
        );
        assert!(
            b.decide(t0, 20, Prio::Chart).is_ok(),
            "the reserve is for charts"
        );
        let later = t0 + Duration::from_secs(60);
        assert!(
            w.decide(later, 20, Prio::Chart).is_ok(),
            "weights older than 60 s drop out"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn header_caps_and_bans() {
        let t0 = Instant::now();
        let mut w = WeightState::default();
        w.note_reply(t0, 200, Some(1790), None);
        assert!(
            w.decide(t0, 20, Prio::Background).is_err(),
            "header 1790 + 20 >= 1800 stops background"
        );
        assert!(
            w.decide(t0, 20, Prio::Chart).is_ok(),
            "the header was cleared and charts may go"
        );
        w.note_reply(t0, 200, Some(2190), None);
        assert!(
            w.decide(t0, 20, Prio::Chart).is_err(),
            "2190 + 20 >= 2200 stops charts"
        );
        let mut b = WeightState::default();
        b.note_reply(t0, 429, None, Some(7));
        assert_eq!(
            b.decide(t0, 10, Prio::Chart),
            Err(Duration::from_secs(5)),
            "wait clamped to 5 s"
        );
        assert!(
            b.decide(t0 + Duration::from_secs(6), 10, Prio::Background)
                .is_err(),
            "still banned at 6 s"
        );
        assert!(
            b.decide(t0 + Duration::from_secs(7), 10, Prio::Chart)
                .is_ok(),
            "free after Retry-After"
        );
        let mut c = WeightState::default();
        c.note_reply(t0, 418, None, None);
        assert!(c
            .decide(t0 + Duration::from_secs(59), 10, Prio::Chart)
            .is_err());
        assert!(
            c.decide(t0 + Duration::from_secs(60), 10, Prio::Chart)
                .is_ok(),
            "60 s without the header"
        );
    }

    /// A real request against a local fake: 429 with Retry-After bans both priorities, a 200's
    /// `X-MBX-USED-WEIGHT-1M` is kept, and the error reads like urllib's.
    #[tokio::test]
    async fn fapi_reply_headers_and_ban() {
        use axum::http::{HeaderMap, HeaderValue, StatusCode};
        use axum::routing::get;
        let app = axum::Router::new()
            .route(
                "/fapi/v1/klines",
                get(|| async {
                    let mut h = HeaderMap::new();
                    h.insert("Retry-After", HeaderValue::from_static("7"));
                    (StatusCode::TOO_MANY_REQUESTS, h, "slow down")
                }),
            )
            .route(
                "/fapi/v1/aggTrades",
                get(|| async {
                    let mut h = HeaderMap::new();
                    h.insert("X-MBX-USED-WEIGHT-1M", HeaderValue::from_static("1795"));
                    (
                        h,
                        r#"[{"a":1,"p":"1.0","q":"2","T":1700000000000,"m":false}]"#,
                    )
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let fapi = Fapi::new(&format!("http://{addr}"), "http://127.0.0.1:9");
        let page = fapi
            .agg_trades("BTCUSDT", None, Prio::Background)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(page.0, vec![1]);
        assert!(
            fapi.weight
                .lock()
                .unwrap()
                .decide(Instant::now(), 10, Prio::Background)
                .is_err(),
            "1795 + 10 >= 1800"
        );
        let err = fapi.klines("BTCUSDT", Tf::M1, 0, 1).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "HTTPError: HTTP Error 429: Too Many Requests"
        );
        let now = Instant::now();
        let mut st = fapi.weight.lock().unwrap();
        assert!(st.decide(now, 10, Prio::Chart).is_err(), "banned after 429");
        assert!(
            st.decide(now + Duration::from_secs(8), 10, Prio::Chart)
                .is_ok(),
            "free after Retry-After"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn futures_list_single_flight_ttl_and_retry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (c, f) = (calls.clone(), fail.clone());
        let list = FuturesList::new(Arc::new(move || {
            let (c, f) = (c.clone(), f.clone());
            Box::pin(async move {
                c.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                if f.load(Ordering::SeqCst) {
                    Err(OfError::value("down"))
                } else {
                    Ok(BTreeSet::from(["XAUUSDT".to_string()]))
                }
            })
        }));
        let gets: Vec<_> = (0..8)
            .map(|_| {
                let l = list.clone();
                tokio::spawn(async move { l.get().await })
            })
            .collect();
        for g in gets {
            assert!(g.await.unwrap().contains("XAUUSDT"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "eight callers, one fetch");
        tokio::time::advance(FUTURES_TTL).await;
        fail.store(true, Ordering::SeqCst);
        assert!(
            list.get().await.contains("XAUUSDT"),
            "a failed refresh keeps the old set"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        tokio::time::advance(Duration::from_secs(299)).await;
        list.get().await;
        assert_eq!(calls.load(Ordering::SeqCst), 2, "no retry before 5 minutes");
        tokio::time::advance(Duration::from_secs(1)).await;
        fail.store(false, Ordering::SeqCst);
        list.get().await;
        assert_eq!(calls.load(Ordering::SeqCst), 3, "retried after 5 minutes");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_three_wire_formats() {
        let k = parse_klines(br#"[[1700000000000,"1.5","2.5","1.0","2.0","10.0",1700000059999,"0",5,"6.25","0","0"]]"#).unwrap();
        assert_eq!(
            k,
            vec![Kline {
                open_ms: 1_700_000_000_000,
                high: 2.5,
                low: 1.0,
                close: 2.0,
                volume: 10.0,
                taker_buy: 6.25
            }]
        );
        let e = parse_exchange_info(br#"{"symbols":[{"symbol":"XAUUSDT","status":"TRADING"},{"symbol":"OLDUSDT","status":"SETTLING"}]}"#).unwrap();
        assert_eq!(
            e.into_iter().collect::<Vec<_>>(),
            vec!["XAUUSDT".to_string()]
        );
        let (ids, t) = parse_agg_trades(br#"[{"a":7,"p":"100.5","q":"0.25","f":1,"l":2,"T":1700000000123,"m":true},{"a":8,"p":"100.6","q":"1","f":3,"l":3,"T":1700000000200,"m":false}]"#).unwrap();
        assert_eq!(ids, vec![7, 8]);
        assert_eq!(t.side, vec![-1, 1]);
        assert_eq!(t.price, vec![100.5, 100.6]);
        assert!(parse_klines(b"{}").is_err());
    }
}
