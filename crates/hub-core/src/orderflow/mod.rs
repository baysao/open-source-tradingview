//! Order flow for any chart symbol: delta per candle, session volume profile, footprint. A port of
//! `hub/orderflow_api.py`, `orderflow_trades.py` and `orderflow.py`; design in
//! `.paul/phases/02-orderflow/02-ARCHITECTURE.md`.
//!
//! `calc` holds every Python number rule and every computation, as pure functions over loaded
//! inputs; that is what the golden fixtures test. Float order matters: inside this module
//! `f64::max`, `f64::min` and `Iterator::sum` are build errors (use `calc::py_max`, `py_min`,
//! `py_sum`), because each one changes bits against Python.

#![deny(clippy::disallowed_methods)]

pub mod binance;
pub mod calc;
pub mod trades;

use hub_types::market::{Source, Tf};
use std::collections::BTreeSet;
use std::fmt;

/// The Python exception class an error is reported as: the 502 body is `"<Class>: <msg>"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PyClass {
    ValueError,
    KeyError,
    IndexError,
    ZeroDivisionError,
    RuntimeError,
    HTTPError,
    URLError,
    TimeoutError,
}

impl PyClass {
    pub fn as_str(self) -> &'static str {
        match self {
            PyClass::ValueError => "ValueError",
            PyClass::KeyError => "KeyError",
            PyClass::IndexError => "IndexError",
            PyClass::ZeroDivisionError => "ZeroDivisionError",
            PyClass::RuntimeError => "RuntimeError",
            PyClass::HTTPError => "HTTPError",
            PyClass::URLError => "URLError",
            PyClass::TimeoutError => "TimeoutError",
        }
    }
}

/// An error with Python's class name and `str(e)`. A `KeyError` message carries its quotes (`'M2'`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfError {
    pub class: PyClass,
    pub msg: String,
}

impl OfError {
    pub fn new(class: PyClass, msg: impl Into<String>) -> OfError {
        OfError {
            class,
            msg: msg.into(),
        }
    }

    pub fn value(msg: impl Into<String>) -> OfError {
        OfError::new(PyClass::ValueError, msg)
    }

    /// `str(e)`, as the warn log line prints it.
    pub fn msg(&self) -> &str {
        &self.msg
    }
}

impl fmt::Display for OfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class.as_str(), self.msg)
    }
}

impl std::error::Error for OfError {}

/// A chart feed that does not exist in this server (D-001).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OffFeed {
    Mt5,
    Arm,
}

impl OffFeed {
    pub fn as_str(self) -> &'static str {
        match self {
            OffFeed::Mt5 => "mt5",
            OffFeed::Arm => "arm",
        }
    }
}

/// The `feed` parameter: Python's `{"mt5","deriv","arm","bybit"}.get(feed, binance_history)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feed {
    Live(Source),
    Off(OffFeed),
}

impl Feed {
    /// Any other value (and a missing one) means Binance, as in Python.
    pub fn from_param(v: Option<&str>) -> Feed {
        match v {
            Some("mt5") => Feed::Off(OffFeed::Mt5),
            Some("arm") => Feed::Off(OffFeed::Arm),
            Some("deriv") => Feed::Live(Source::Deriv),
            Some("bybit") => Feed::Live(Source::Bybit),
            _ => Feed::Live(Source::Binance),
        }
    }

    /// D-009: what a history call on a feed this server does not have answers.
    pub fn off_error(off: OffFeed) -> OfError {
        OfError::new(
            PyClass::RuntimeError,
            format!(
                "{} history is not available in this server (D-001)",
                off.as_str()
            ),
        )
    }
}

/// The chart a request is about. `raw` is the ticker as the page sent it: `real_symbol` reads it
/// (gold maps to `XAUUSDT`), while the history call parses it per feed (`Ticker::parse`), which maps
/// Binance gold to `PAXGUSDT`. Keeping both is the R32 rule in a type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Chart {
    pub raw: String,
    pub feed: Feed,
}

/// The raw `tf` parameter, resolved where Python first indexes `TF_SEC[tf]`, so the error order
/// matches (in `delta` a no-real-data `ValueError` comes before a bad-tf `KeyError`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TfArg(pub String);

impl TfArg {
    pub fn resolve(&self) -> Result<Tf, OfError> {
        Tf::parse(&self.0).ok_or_else(|| OfError::new(PyClass::KeyError, calc::py_repr(&self.0)))
    }
}

/// The `source` parameter: `auto`, `real`, `estimated`; anything else behaves as `auto`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pick {
    Auto,
    Real,
    Estimated,
}

impl Pick {
    pub fn from_param(v: Option<&str>) -> Pick {
        match v {
            Some("real") => Pick::Real,
            Some("estimated") => Pick::Estimated,
            _ => Pick::Auto,
        }
    }
}

// ---------------------------------------------------------------- 02-02: the service

use crate::bars_cache::{BarsCache, Freshness};
use crate::clock::WallClock;
use crate::market::{BarsQuery, FeedError, Ticker};
use crate::sources::Endpoints;
use calc::{
    auto_row_size, delta, delta_window, estimated_rows, footprint, footprint_spans, intrabar,
    need_volume, profile, profile_window, py_float, py_int, real_rows, resolve, rows_merge,
    rows_need, rows_window, shift, FootprintArgs, FootprintData, LowerRows, ProfileOpts, Span,
    TradeWindow, MAX_ROWS,
};
use futures_util::future::BoxFuture;
use hub_types::market::Bar;
use hub_types::orderflow::{DeltaReply, FlowSource, FootprintReply, Num, ProfileReply};
use serde::Serialize;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Which route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Delta,
    Profile,
    Footprint,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Delta => "delta",
            Kind::Profile => "profile",
            Kind::Footprint => "footprint",
        })
    }
}

/// What every route has, after the parts Python parses in the route (`int(from)`, `int(to)`).
#[derive(Clone, Debug, PartialEq)]
pub struct Common {
    pub chart: Chart,
    pub tf: TfArg,
    pub from_s: i64,
    pub to_s: i64,
    pub pick: Pick,
}

/// Footprint options. `row_size` stays raw: Python parses it after the route's history call.
#[derive(Clone, Debug, PartialEq)]
pub struct FootprintOpts {
    pub row_size: Option<String>,
    pub fill: bool,
}

/// A parsed order-flow request.
#[derive(Clone, Debug, PartialEq)]
pub enum Ask {
    Delta(Common),
    Profile(Common, ProfileOpts),
    Footprint(Common, FootprintOpts),
}

impl Ask {
    /// The route's argument parsing, in Python's order, with Python's defaults and error texts.
    /// A blank value counts as absent (`parse_qs` drops blanks).
    pub fn parse(kind: Kind, q: &HashMap<String, String>, now_s: f64) -> Result<Ask, OfError> {
        let get = |k: &str| q.get(k).map(String::as_str).filter(|v| !v.is_empty());
        let now = now_s as i64;
        let back = if kind == Kind::Footprint {
            3 * 3600
        } else {
            86_400
        };
        let from_s = get("from").map_or(Ok(now - back), py_int)?;
        let to_s = get("to").map_or(Ok(now), py_int)?;
        let common = Common {
            chart: Chart {
                raw: get("symbol").unwrap_or("frxXAUUSD").to_string(),
                feed: Feed::from_param(get("feed")),
            },
            tf: TfArg(get("tf").unwrap_or("M5").to_string()),
            from_s,
            to_s,
            pick: Pick::from_param(get("source")),
        };
        Ok(match kind {
            Kind::Delta => Ask::Delta(common),
            Kind::Profile => {
                let rows = get("rows").map_or(Ok(24), py_int)?;
                let row_size = get("row_size").map_or(Ok(0.0), py_float)?;
                let va = get("va").map_or(Ok(70.0), py_float)?;
                // D-009 guards: Python hangs on a huge row count and writes invalid JSON for NaN.
                if rows > 100_000 {
                    return Err(OfError::value("rows must be 100000 or fewer"));
                }
                if !row_size.is_finite() || !va.is_finite() {
                    return Err(OfError::value("row_size and va must be finite numbers"));
                }
                Ask::Profile(
                    common,
                    ProfileOpts {
                        mode: get("mode").unwrap_or("sessions").to_string(),
                        rows,
                        row_size,
                        va_pct: va / 100.0,
                        developing: get("developing").unwrap_or("1") == "1",
                    },
                )
            }
            Kind::Footprint => Ask::Footprint(
                common,
                FootprintOpts {
                    row_size: get("row_size").map(str::to_string),
                    fill: get("fill").unwrap_or("1") != "0",
                },
            ),
        })
    }
}

/// A route body, serialized with Python's keys.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Delta(DeltaReply),
    Profile(ProfileReply),
    Footprint(FootprintReply),
}

/// Where footprint trade windows come from (`store(sym).trades(t0, t1)`): the trade store, or a
/// test's recorded windows.
pub trait TradeWindows: Send + Sync + 'static {
    fn window<'a>(&'a self, sym: &'a str, t0_ms: i64, t1_ms: i64) -> BoxFuture<'a, TradeWindow>;
}

impl TradeWindows for trades::Trades {
    fn window<'a>(&'a self, sym: &'a str, t0_ms: i64, t1_ms: i64) -> BoxFuture<'a, TradeWindow> {
        Box::pin(trades::Trades::window(self, sym, t0_ms, t1_ms))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RowsKey {
    src: FlowSource,
    raw: String,
    tf: Tf,
    /// Python's key has no feed; with it, the same ticker on two feeds cannot share rows (D-009).
    feed: Feed,
}

/// Python's `_cache`: closed lower bars per key, bounded (LRU over keys, D-009). Planning and merge
/// happen under the lock; fetches never hold it.
struct RowsCache {
    map: Mutex<lru::LruCache<RowsKey, LowerRows>>,
}

impl RowsCache {
    fn new() -> RowsCache {
        RowsCache {
            map: Mutex::new(lru::LruCache::new(
                NonZeroUsize::new(256).expect("non-zero"),
            )),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, lru::LruCache<RowsKey, LowerRows>> {
        self.map.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// An upstream failure with the Python class its text belongs to (D-009 note).
fn from_feed(e: FeedError) -> OfError {
    let m = e.to_string();
    let class = if m.starts_with("HTTP Error") {
        PyClass::HTTPError
    } else if m.starts_with("<urlopen error") {
        PyClass::URLError
    } else if m.contains("timed out") {
        PyClass::TimeoutError
    } else {
        PyClass::RuntimeError
    };
    OfError::new(class, m)
}

/// The order-flow engine: delta, profile and footprint for any chart symbol. Cheap to clone.
#[derive(Clone)]
pub struct OrderFlow {
    bars: BarsCache,
    fapi: binance::Fapi,
    futures: binance::FuturesList,
    trades: Arc<dyn TradeWindows>,
    rows: Arc<RowsCache>,
    clock: WallClock,
}

impl OrderFlow {
    /// The real engine: Binance futures and the archive from `ep`, chart history from `bars`, the
    /// trade store with its cache in `cache_dir` (`None`: memory only). No I/O happens here.
    pub fn new(
        bars: BarsCache,
        ep: &Endpoints,
        clock: WallClock,
        cache_dir: Option<PathBuf>,
    ) -> OrderFlow {
        let fapi = binance::Fapi::new(&ep.binance_fapi, &ep.binance_vision);
        let futures = binance::FuturesList::from_fapi(fapi.clone());
        let trades = Arc::new(trades::Trades::new(
            Arc::new(fapi.clone()),
            clock,
            cache_dir,
        ));
        OrderFlow::with_parts(bars, fapi, futures, trades, clock)
    }

    /// The engine from its parts (tests replace the futures list and the trade windows).
    pub fn with_parts(
        bars: BarsCache,
        fapi: binance::Fapi,
        futures: binance::FuturesList,
        trades: Arc<dyn TradeWindows>,
        clock: WallClock,
    ) -> OrderFlow {
        OrderFlow {
            bars,
            fapi,
            futures,
            trades,
            rows: Arc::new(RowsCache::new()),
            clock,
        }
    }

    /// `time.time()`.
    pub fn now_s(&self) -> f64 {
        self.clock.now_s()
    }

    pub async fn answer(&self, ask: &Ask) -> Result<Reply, OfError> {
        match ask {
            Ask::Delta(c) => self.delta(c).await.map(Reply::Delta),
            Ask::Profile(c, o) => self.profile(c, o).await.map(Reply::Profile),
            Ask::Footprint(c, o) => self.footprint(c, o).await.map(Reply::Footprint),
        }
    }

    async fn delta(&self, c: &Common) -> Result<DeltaReply, OfError> {
        let fut = self.futures.get().await;
        let r = resolve(&c.chart.raw, c.pick, &fut)?;
        let tf = c.tf.resolve()?;
        let lower = self
            .rows(
                r.src,
                &c.chart,
                intrabar(tf),
                delta_window(tf, c.from_s, c.to_s),
                &fut,
            )
            .await?;
        if r.src == FlowSource::Estimated {
            need_volume(&lower)?;
        }
        Ok(delta(tf, &lower, &r))
    }

    async fn profile(&self, c: &Common, o: &ProfileOpts) -> Result<ProfileReply, OfError> {
        let fut = self.futures.get().await;
        let r = resolve(&c.chart.raw, c.pick, &fut)?;
        let tf = c.tf.resolve()?;
        let w = profile_window(tf, &o.mode, r.src, c.from_s, c.to_s);
        let lower = self.rows(r.src, &c.chart, w.itf, w.lower, &fut).await?;
        let chart = match (r.src, w.chart) {
            (FlowSource::Real, Some(span)) => Some(
                self.rows(FlowSource::Estimated, &c.chart, tf, span, &fut)
                    .await?,
            ),
            _ => {
                need_volume(&lower)?;
                None
            }
        };
        profile(&w, o, &r, &lower, chart.as_ref())
    }

    async fn footprint(&self, c: &Common, o: &FootprintOpts) -> Result<FootprintReply, OfError> {
        let tf = c.tf.resolve()?;
        let (a, b) = (c.from_s, c.to_s);
        let sec = tf.secs();
        let n = 1000.min(5.max((b - a).div_euclid(sec) + 5));
        let bars = self.history(&c.chart, tf, n, b * 1000).await?;
        let cb: Vec<(i64, f64, f64)> = bars
            .iter()
            .filter(|x| a <= x.time && x.time <= b)
            .map(|x| (x.time, x.high, x.low))
            .collect();
        let row_size_f = o.row_size.as_deref().map_or(Ok(0.0), py_float)?;
        if !row_size_f.is_finite() {
            return Err(OfError::value("row_size must be a finite number"));
        }
        let fut = self.futures.get().await;
        let r = resolve(&c.chart.raw, c.pick, &fut)?;
        let row_size = if row_size_f <= 0.0 {
            auto_row_size(&cb)
        } else {
            Num::Float(row_size_f)
        };
        let args = FootprintArgs {
            tf,
            from_s: a,
            to_s: b,
            row_size,
            fill: o.fill,
        };
        let (m1_span, chart_span, (t0, t1)) = footprint_spans(tf, a, b);
        match (r.src, r.real.as_deref()) {
            (FlowSource::Real, Some(sym)) => {
                let m1 = self
                    .rows(FlowSource::Real, &c.chart, Tf::M1, m1_span, &fut)
                    .await?;
                let chart = self
                    .rows(FlowSource::Estimated, &c.chart, tf, chart_span, &fut)
                    .await?;
                let g_at = shift(&m1, &chart, sec, 60);
                let window = self.trades.window(sym, t0, t1).await;
                footprint(
                    &args,
                    FootprintData::Real {
                        m1: &m1,
                        g_at: &g_at,
                        window: &window,
                    },
                    self.now_s(),
                    &r,
                )
            }
            _ => {
                let m1 = self
                    .rows(FlowSource::Estimated, &c.chart, Tf::M1, m1_span, &fut)
                    .await?;
                need_volume(&m1)?;
                footprint(
                    &args,
                    FootprintData::Estimated { m1: &m1 },
                    self.now_s(),
                    &r,
                )
            }
        }
    }

    /// `_rows`: plan under the lock, fetch each range with no lock held, merge each range as it
    /// arrives (a failure on the second range keeps the first, as Python).
    async fn rows(
        &self,
        src: FlowSource,
        chart: &Chart,
        tf: Tf,
        span: Span,
        fut: &BTreeSet<String>,
    ) -> Result<LowerRows, OfError> {
        let key = RowsKey {
            src,
            raw: chart.raw.clone(),
            tf,
            feed: chart.feed,
        };
        let need = {
            let mut m = self.rows.lock();
            let have = m.get_or_insert_mut(key.clone(), LowerRows::new);
            rows_need(have, tf.secs(), span.a, span.b)
        };
        for s in need {
            let got = match src {
                FlowSource::Real => {
                    let sym = calc::real_symbol(&chart.raw, fut).ok_or_else(|| {
                        OfError::value(format!("no real buyer/seller data for {}", chart.raw))
                    })?;
                    self.binance_rows(&sym, tf, s).await?
                }
                FlowSource::Estimated => self.estimated_rows(chart, tf, s).await?,
            };
            let mut m = self.rows.lock();
            rows_merge(m.get_or_insert_mut(key.clone(), LowerRows::new), got);
        }
        let mut m = self.rows.lock();
        Ok(rows_window(m.get_or_insert_mut(key, LowerRows::new), span))
    }

    /// `_binance_rows`: klines pages of 1,500 from `a` while the start is within `b`; stop on an
    /// empty or short page; the next start is the last open plus one interval.
    async fn binance_rows(&self, sym: &str, itf: Tf, s: Span) -> Result<LowerRows, OfError> {
        let (mut start, step, end) = (s.a * 1000, itf.secs() * 1000, s.b * 1000);
        let mut pages = Vec::new();
        while start <= end {
            let page = self.fapi.klines(sym, itf, start, end).await?;
            let Some(last) = page.last() else { break };
            start = last.open_ms + step;
            let short = page.len() < 1500;
            pages.push(page);
            if short {
                break;
            }
        }
        Ok(real_rows(&pages))
    }

    /// `_estimated_rows`: history chunks newest first until the span start, a short chunk, or
    /// more than `MAX_ROWS` bars.
    async fn estimated_rows(&self, chart: &Chart, itf: Tf, s: Span) -> Result<LowerRows, OfError> {
        let sec = itf.secs();
        let mut to_ms = (s.b + sec) * 1000;
        let need = (s.b - s.a).div_euclid(sec) + 3;
        let (mut chunks, mut total): (Vec<Vec<Bar>>, i64) = (Vec::new(), 0);
        loop {
            let n = 3.max(5000.min(need - total));
            let chunk = self.history(chart, itf, n, to_ms).await?;
            if chunk.is_empty() {
                break;
            }
            total += chunk.len() as i64;
            let (first, len) = (chunk[0].time, chunk.len());
            chunks.push(chunk);
            if first <= s.a || len < 50 {
                break;
            }
            to_ms = (first - sec) * 1000;
            if total > MAX_ROWS {
                break;
            }
        }
        Ok(estimated_rows(&chunks, s))
    }

    /// The chart feed's `history(ticker, tf, n, to_ms)` through the hub's `BarsCache`. A page that
    /// reaches the forming bar is refetched (`Fresh`); older pages may come from the cache.
    async fn history(
        &self,
        chart: &Chart,
        tf: Tf,
        n: i64,
        to_ms: i64,
    ) -> Result<Vec<Bar>, OfError> {
        let source = match chart.feed {
            Feed::Live(s) => s,
            Feed::Off(off) => return Err(Feed::off_error(off)),
        };
        let ticker = Ticker::parse(source, &chart.raw).map_err(from_feed)?;
        let q = BarsQuery::new(source, ticker, tf, n, Some(to_ms));
        let forming = to_ms as f64 >= self.clock.now_ms() - 2.0 * (tf.secs() * 1000) as f64;
        let fresh = if forming {
            Freshness::Fresh
        } else {
            Freshness::Cached
        };
        Ok(self
            .bars
            .get(&q, fresh)
            .await
            .map_err(from_feed)?
            .as_ref()
            .clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hub_types::market::Bar;

    #[test]
    fn errors_print_like_python() {
        assert_eq!(
            TfArg("M2".into()).resolve().unwrap_err().to_string(),
            "KeyError: 'M2'"
        );
        assert_eq!(TfArg("M5".into()).resolve().unwrap(), Tf::M5);
        assert_eq!(
            Feed::off_error(OffFeed::Mt5).to_string(),
            "RuntimeError: mt5 history is not available in this server (D-001)"
        );
        assert_eq!(Feed::from_param(None), Feed::Live(Source::Binance));
        assert_eq!(
            Feed::from_param(Some("whatever")),
            Feed::Live(Source::Binance)
        );
        assert_eq!(Feed::from_param(Some("arm")), Feed::Off(OffFeed::Arm));
    }

    /// History stand-in: counts upstream calls per source, answers 60 one-minute bars ending at
    /// `to_ms` with volume, so estimated rows have something to sign.
    struct Counting(std::sync::Mutex<Vec<(Source, i64)>>);

    impl crate::sources::HistoryFetch for Counting {
        fn fetch<'a>(&'a self, q: &'a BarsQuery) -> BoxFuture<'a, Result<Vec<Bar>, FeedError>> {
            Box::pin(async move {
                self.0.lock().unwrap().push((q.source, q.limit));
                let end = q.to_ms.unwrap_or(0) / 1000;
                Ok((0..60)
                    .map(|i| {
                        let t = end - (59 - i) * 60;
                        Bar {
                            time: t - t.rem_euclid(60),
                            open: 1.0,
                            high: 2.0,
                            low: 0.5,
                            close: 1.0 + (i % 3) as f64,
                            volume: 3.0,
                        }
                    })
                    .collect())
            })
        }
    }

    fn engine(now_ms: f64) -> (OrderFlow, Arc<Counting>) {
        let up = Arc::new(Counting(std::sync::Mutex::default()));
        let fut = binance::FuturesList::new(Arc::new(|| Box::pin(async { Ok(BTreeSet::new()) })));
        let of = OrderFlow::with_parts(
            BarsCache::new(up.clone()),
            binance::Fapi::new("http://127.0.0.1:9", "http://127.0.0.1:9"),
            fut,
            Arc::new(trades::Trades::new(
                Arc::new(binance::Fapi::new(
                    "http://127.0.0.1:9",
                    "http://127.0.0.1:9",
                )),
                WallClock::frozen(now_ms),
                None,
            )),
            WallClock::frozen(now_ms),
        );
        (of, up)
    }

    #[tokio::test]
    async fn forming_pages_are_refetched_and_old_pages_cached() {
        let now_ms = 1_795_003_200_000.0;
        let (of, up) = engine(now_ms);
        let chart = Chart {
            raw: "BTCUSD".into(),
            feed: Feed::Live(Source::Binance),
        };
        for _ in 0..2 {
            of.history(&chart, Tf::M1, 100, now_ms as i64)
                .await
                .unwrap();
        }
        assert_eq!(
            up.0.lock().unwrap().len(),
            2,
            "a page reaching the forming bar is fetched every time"
        );
        for _ in 0..2 {
            of.history(&chart, Tf::M1, 100, now_ms as i64 - 3_600_000)
                .await
                .unwrap();
        }
        assert_eq!(
            up.0.lock().unwrap().len(),
            3,
            "an older page is served from the cache"
        );
        let off = Chart {
            raw: "RMT_XAUUSD".into(),
            feed: Feed::Off(OffFeed::Mt5),
        };
        assert_eq!(
            of.history(&off, Tf::M1, 10, 0).await.unwrap_err().class,
            PyClass::RuntimeError
        );
    }

    #[tokio::test]
    async fn the_same_ticker_on_two_feeds_has_its_own_rows() {
        let now_ms = 1_795_003_200_000.0;
        let (of, up) = engine(now_ms);
        let span = Span {
            a: 1_795_000_000,
            b: 1_795_001_200,
        };
        for feed in [Source::Binance, Source::Bybit] {
            let chart = Chart {
                raw: "BTCUSDT".into(),
                feed: Feed::Live(feed),
            };
            of.rows(
                FlowSource::Estimated,
                &chart,
                Tf::M1,
                span,
                &BTreeSet::new(),
            )
            .await
            .unwrap();
        }
        let calls = up.0.lock().unwrap().clone();
        let first = |src: Source| calls.iter().find(|(s, _)| *s == src).map(|(_, n)| *n);
        assert!(first(Source::Bybit).is_some(), "bybit loads: {calls:?}");
        assert_eq!(
            first(Source::Bybit),
            first(Source::Binance),
            "the second feed loads the whole span, not only the last cached bar: {calls:?}"
        );
    }

    #[test]
    fn ask_parse_keeps_python_defaults_and_errors() {
        let q = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>()
        };
        let now = 1_795_003_200.0;
        let Ask::Footprint(c, o) =
            Ask::parse(Kind::Footprint, &q(&[("row_size", "abc")]), now).unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (c.from_s, c.to_s, c.chart.raw.as_str(), c.tf.0.as_str()),
            (1_795_003_200 - 3 * 3600, 1_795_003_200, "frxXAUUSD", "M5")
        );
        assert_eq!(
            o.row_size.as_deref(),
            Some("abc"),
            "footprint parses row_size after the history call"
        );
        let Ask::Profile(_, p) = Ask::parse(
            Kind::Profile,
            &q(&[("va", "68"), ("developing", "0"), ("from", "")]),
            now,
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(
            (p.rows, p.va_pct, p.developing, p.mode.as_str()),
            (24, 0.68, false, "sessions")
        );
        assert_eq!(
            Ask::parse(Kind::Delta, &q(&[("from", "abc")]), now)
                .unwrap_err()
                .to_string(),
            "ValueError: invalid literal for int() with base 10: 'abc'"
        );
        assert_eq!(
            Ask::parse(Kind::Profile, &q(&[("rows", "x"), ("va", "y")]), now)
                .unwrap_err()
                .to_string(),
            "ValueError: invalid literal for int() with base 10: 'x'",
            "rows is parsed before va"
        );
        assert_eq!(Kind::Footprint.to_string(), "footprint");
    }
}
