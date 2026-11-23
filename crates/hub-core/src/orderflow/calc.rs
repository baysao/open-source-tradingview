//! Every Python number rule and every order-flow computation, as pure functions over loaded inputs.
//!
//! Python's semantics live in the `py_*` functions below; nothing else in this module adds,
//! compares or rounds floats in its own way. Loops keep Python's iteration order, because the order
//! of float additions decides the last bit and `round(x, 4)` can make that bit visible.

use super::{OfError, Pick, PyClass};
use hub_types::market::{Bar, Tf};
use hub_types::orderflow::{
    Candle, DeltaReply, DeltaRow, DevPoint, FlowSource, FootprintReply, Num, ProfileReply, Session,
    Vol,
};
use indexmap::IndexMap;
use std::collections::{BTreeMap, BTreeSet};

// ---------------------------------------------------------------- Python numerics

/// Python `round(x, nd)` for a float: the correctly rounded decimal with `nd` places (ties to even
/// on the exact binary value), read back as the nearest double. Non-finite values come back as is.
pub fn py_round(x: f64, nd: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.nd$}").parse().unwrap_or(x)
}

/// Python's builtin `sum()` over floats (CPython 3.12 and later): the first item is added to the
/// int 0, the rest with Neumaier compensation, and the compensation is added once at the end when it
/// is finite and not zero. An empty input is 0.0 (Python returns the int 0; no caller sums nothing).
pub fn py_sum(xs: impl IntoIterator<Item = f64>) -> f64 {
    let mut it = xs.into_iter();
    let Some(first) = it.next() else { return 0.0 };
    let mut f = 0.0 + first; // int 0 + x: -0.0 becomes 0.0, as in Python
    let mut c = 0.0;
    for x in it {
        let t = f + x;
        if f.abs() >= x.abs() {
            c += (f - t) + x;
        } else {
            c += (x - t) + f;
        }
        f = t;
    }
    if c != 0.0 && c.is_finite() {
        f += c;
    }
    f
}

/// Python float `%`: `fmod`, then the sign of the divisor; a zero result takes the divisor's sign.
pub fn py_mod(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) {
            m + b
        } else {
            m
        }
    } else {
        0.0_f64.copysign(b)
    }
}

/// Python `max(a, b)`: `b` only when `b > a`, so the first argument wins ties (`max(0.0, -0.0)` is `0.0`).
pub fn py_max(a: f64, b: f64) -> f64 {
    if b > a {
        b
    } else {
        a
    }
}

/// Python `min(a, b)`: `b` only when `b < a`.
pub fn py_min(a: f64, b: f64) -> f64 {
    if b < a {
        b
    } else {
        a
    }
}

/// Python `sorted()` on floats: stable, ordered by `<` only (`-0.0` and `0.0` keep their order).
pub fn py_sort(v: &mut [f64]) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

/// `math.floor(x)` of a float, as an int.
pub fn py_floor(x: f64) -> i64 {
    x.floor() as i64
}

/// Python `repr()` of a str, as error messages quote it: single quotes unless the text holds a
/// single quote and no double quote; backslash, the quote and control characters escaped.
pub fn py_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(q);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32))
            }
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

/// PEP 515 underscores: allowed only between two digits. Returns the text without them, or `None`
/// when one is misplaced.
fn strip_underscores(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in b.iter().enumerate() {
        if c == b'_' {
            let prev = i.checked_sub(1).map(|j| b[j]);
            let next = b.get(i + 1).copied();
            if !(prev.is_some_and(|p| p.is_ascii_digit())
                && next.is_some_and(|n| n.is_ascii_digit()))
            {
                return None;
            }
        } else {
            out.push(c as char);
        }
    }
    Some(out)
}

/// Python `int(s)` for a query string (base 10): surrounding whitespace, an optional sign, ASCII
/// digits with single underscores between them.
pub fn py_int(s: &str) -> Result<i64, OfError> {
    let bad = || {
        OfError::value(format!(
            "invalid literal for int() with base 10: {}",
            py_repr(s)
        ))
    };
    let t = s.trim();
    let digits = t.strip_prefix(['+', '-']).unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit() || c == b'_') {
        return Err(bad());
    }
    let clean = strip_underscores(t).ok_or_else(bad)?;
    clean.parse::<i64>().map_err(|_| bad())
}

/// Python `float(s)`: surrounding whitespace, PEP 515 underscores, `inf`, `infinity`, `nan` in any
/// case with an optional sign.
pub fn py_float(s: &str) -> Result<f64, OfError> {
    let bad = || OfError::value(format!("could not convert string to float: {}", py_repr(s)));
    let t = s.trim();
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    let neg = t.starts_with('-');
    match body.to_ascii_lowercase().as_str() {
        "inf" | "infinity" => {
            return Ok(if neg {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            })
        }
        "nan" => return Ok(f64::NAN),
        _ => {}
    }
    if body.is_empty()
        || !body
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-' | b'_'))
    {
        return Err(bad());
    }
    let clean = strip_underscores(t).ok_or_else(bad)?;
    clean.parse::<f64>().map_err(|_| bad())
}

/// `nice_step`: round a row size up to 1, 2, 2.5 or 5 x 10^k. `e = 10 ** math.floor(math.log10(x))`
/// is a Python int for k >= 0 (so `m * e` stays an int except for 2.5) and a float for k < 0
/// (`10 ** -k` goes through C `pow`).
pub fn nice_step(x: f64) -> Num {
    if x <= 0.0 {
        return Num::Float(1.0);
    }
    let k = py_floor(x.log10());
    // Python ints are unbounded; i64 holds every value up to k = 17 (10 * 10^17). Row sizes that
    // large do not occur for prices, so larger k falls to the float branch.
    if (0..=17).contains(&k) {
        let e = 10_i64.pow(k as u32);
        for m in [1_i64, 2] {
            if x <= (m * e) as f64 {
                return Num::Int(m * e);
            }
        }
        let v = 2.5 * e as f64;
        if x <= v {
            return Num::Float(v);
        }
        for m in [5_i64, 10] {
            if x <= (m * e) as f64 {
                return Num::Int(m * e);
            }
        }
        Num::Int(10 * e)
    } else {
        let e = 10.0_f64.powf(k as f64);
        for m in [1.0, 2.0, 2.5, 5.0, 10.0] {
            let v = m * e;
            if x <= v {
                return Num::Float(v);
            }
        }
        Num::Float(10.0 * e)
    }
}

// ---------------------------------------------------------------- data the computations run on

/// Python's `(buy, sell, sign, high, low, close)` tuple for one lower bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lower {
    pub buy: f64,
    pub sell: f64,
    pub sign: i8,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// Python's `{t: Lower}` dict. INSERTION ORDER IS PART OF THE VALUE: the footprint fill adds into a
/// cell in this order, and the order comes from the history of cache updates (`have.update(got)`
/// keeps an existing key's place and appends new keys). `IndexMap::insert` has exactly those
/// semantics. Never sort it in place; sort a copy of the keys where Python calls `sorted()`.
pub type LowerRows = IndexMap<i64, Lower>;

/// One Binance futures 1-minute (or other interval) kline, parsed from the wire array.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kline {
    pub open_ms: i64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub taker_buy: f64,
}

/// Trades as columns, in the store's concatenation order (day files by date, then segments).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TradeCols {
    pub t_ms: Vec<i64>,
    pub price: Vec<f64>,
    pub qty: Vec<f64>,
    pub side: Vec<i8>,
}

impl TradeCols {
    pub fn len(&self) -> usize {
        self.t_ms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.t_ms.is_empty()
    }
}

/// What `TradeStore.trades(t0, t1)` returns: the fully loaded trades inside the range, the exact
/// time ranges they cover (already unioned), and whether more is coming.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TradeWindow {
    pub trades: TradeCols,
    pub covered: Vec<(i64, i64)>,
    pub pending: bool,
}

/// The source picked for a request, and `real_symbol(ticker)` (reported in every reply).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub src: FlowSource,
    pub real: Option<String>,
}

/// Inclusive span in seconds: what Python passes to `_rows(source, ticker, tf, a, b)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub a: i64,
    pub b: i64,
}

pub const MAX_ROWS: i64 = 20_000;
/// Guards where Python would hang or run out of memory (D-009): price rows in one footprint
/// candle and in one profile histogram. No answer Python can give is changed by them.
pub const MAX_CANDLE_ROWS: i64 = 1_000_000;
pub const MAX_PROFILE_ROWS: i64 = 1_000_000;

fn too_many_rows() -> OfError {
    OfError::value("more than 1000000 price rows: choose a larger row_size")
}
pub const POLL_S: f64 = 4.0;

const UNIT_REAL: &str = "base volume (coin / oz)";
const UNIT_EST: &str = "feed volume (MT5 = ticks)";
const METHOD_REAL: &str = "Binance taker buy volume";
const METHOD_EST: &str = "lower-bar direction (TradingView default)";

fn unit(src: FlowSource) -> String {
    match src {
        FlowSource::Real => UNIT_REAL,
        FlowSource::Estimated => UNIT_EST,
    }
    .to_string()
}

/// Lower timeframe used inside each chart bar (`INTRABAR`).
pub fn intrabar(tf: Tf) -> Tf {
    match tf {
        Tf::M1 | Tf::M5 | Tf::M15 => Tf::M1,
        Tf::M30 | Tf::H1 => Tf::M5,
        Tf::H4 => Tf::M15,
        Tf::D1 => Tf::H1,
    }
}

/// `t - t % sec`, or `t - t % 86400` on D1 (the same here, kept for the reader).
pub fn bucket(tf: Tf, t: i64) -> i64 {
    let sec = if tf == Tf::D1 { 86_400 } else { tf.secs() };
    t - t.rem_euclid(sec)
}

// ---------------------------------------------------------------- symbols and source

/// Chart ticker to the Binance futures symbol with real buyer and seller data, or `None`.
pub fn real_symbol(raw: &str, futures: &BTreeSet<String>) -> Option<String> {
    let up = raw.to_uppercase();
    let mut t: &str = up.rsplit(':').next().unwrap_or("");
    for p in ["RMT_", "BYBIT_S_", "BYBIT_I_", "BYBIT_", "FRX", "CRY"] {
        if let Some(rest) = t.strip_prefix(p) {
            t = rest;
            break;
        }
    }
    if t.ends_with(".MARK") || t.ends_with(".INDEX") {
        return None;
    }
    let t = if t.ends_with("USD247") {
        &t[..t.len() - 3]
    } else {
        t
    };
    let base = t.strip_suffix("USDT").or_else(|| t.strip_suffix("USD"))?;
    let base = match base {
        "XCU" => "COPPER",
        "XNG" => "NATGAS",
        b => b,
    };
    let s = format!("{base}USDT");
    futures.contains(&s).then_some(s)
}

/// `pick_source` plus the `ValueError` the three engines raise when it returns `None`.
pub fn resolve(raw: &str, pick: Pick, futures: &BTreeSet<String>) -> Result<Resolved, OfError> {
    let real = real_symbol(raw, futures);
    let src = match pick {
        Pick::Real if real.is_some() => FlowSource::Real,
        Pick::Real => {
            return Err(OfError::value(format!(
                "no real buyer/seller data for {raw}"
            )))
        }
        Pick::Estimated => FlowSource::Estimated,
        Pick::Auto if real.is_some() => FlowSource::Real,
        Pick::Auto => FlowSource::Estimated,
    };
    Ok(Resolved { src, real })
}

// ---------------------------------------------------------------- the lower-bar cache rules

/// Python's `_rows` planning: the ranges to fetch for `[a, b]` given what `have` holds. At most two:
/// before the cached start, and from the last cached bar (re-read: it may still be forming) to `b`.
/// A hole inside the cached span is never refetched (Python quirk, kept: D-009). Ranges with
/// `hi < lo` are dropped, as Python's `continue` does.
pub fn rows_need(have: &LowerRows, sec: i64, a: i64, b: i64) -> Vec<Span> {
    let first = have.keys().copied().min();
    let last = have.keys().copied().max();
    let mut need = Vec::new();
    match first {
        None => need.push(Span { a, b }),
        Some(f) if a < f => need.push(Span {
            a,
            b: if f - sec < b { f - sec } else { b },
        }),
        Some(_) => {}
    }
    if let Some(l) = last {
        if b > l - sec {
            need.push(Span { a: a.max(l), b });
        }
    }
    need.retain(|s| s.b >= s.a);
    need
}

/// `have.update(got)`: existing keys keep their place, new keys are appended in `got`'s order.
pub fn rows_merge(have: &mut LowerRows, got: LowerRows) {
    for (t, v) in got {
        have.insert(t, v);
    }
}

/// `{t: v for t, v in have.items() if a <= t <= b}`: insertion order kept.
pub fn rows_window(have: &LowerRows, span: Span) -> LowerRows {
    have.iter()
        .filter(|(t, _)| span.a <= **t && **t <= span.b)
        .map(|(t, v)| (*t, *v))
        .collect()
}

/// `_binance_rows` body: klines pages in fetch order into rows (sign 0, buy = taker buy,
/// sell = volume - taker buy).
pub fn real_rows(pages: &[Vec<Kline>]) -> LowerRows {
    let mut out = LowerRows::new();
    for k in pages.iter().flatten() {
        out.insert(
            k.open_ms.div_euclid(1000),
            Lower {
                buy: k.taker_buy,
                sell: k.volume - k.taker_buy,
                sign: 0,
                high: k.high,
                low: k.low,
                close: k.close,
            },
        );
    }
    out
}

/// `_estimated_rows` body: `chunks` in fetch order (newest first); Python joins them as
/// `chunk + bars`, so older chunks come first. Stable sort by time, the first bar of each time kept,
/// the candle direction carried across bars (a flat bar copies the direction of the previous
/// close), and only bars inside `span` kept.
pub fn estimated_rows(chunks: &[Vec<Bar>], span: Span) -> LowerRows {
    let mut bars: Vec<Bar> = Vec::new();
    for c in chunks {
        let mut joined = c.clone();
        joined.extend(bars);
        bars = joined;
    }
    bars.sort_by_key(|x| x.time);
    let mut seen = std::collections::HashSet::new();
    let (mut out, mut prev_close, mut sign) = (LowerRows::new(), None::<f64>, 0_i8);
    for x in bars {
        if !seen.insert(x.time) {
            continue;
        }
        let (o, c, v) = (x.open, x.close, x.volume);
        if c > o {
            sign = 1;
        } else if c < o {
            sign = -1;
        } else if let Some(p) = prev_close {
            if c != p {
                sign = if c > p { 1 } else { -1 };
            }
        }
        prev_close = Some(c);
        if span.a <= x.time && x.time <= span.b {
            let (buy, sell) = match sign {
                s if s > 0 => (v, 0.0),
                s if s < 0 => (0.0, v),
                _ => (0.0, 0.0),
            };
            out.insert(
                x.time,
                Lower {
                    buy,
                    sell,
                    sign,
                    high: x.high,
                    low: x.low,
                    close: c,
                },
            );
        }
    }
    out
}

/// `_need_volume`: a feed with no volume and no Binance match would draw zeros everywhere.
pub fn need_volume(rows: &LowerRows) -> Result<(), OfError> {
    if !rows.is_empty() && !rows.values().any(|v| v.buy != 0.0 || v.sell != 0.0) {
        return Err(OfError::value(
            "this feed sends no volume and has no Binance match, so there is no order flow to show",
        ));
    }
    Ok(())
}

/// `_shift`: the gap (chart close minus Binance close) for every lower bar, as the upper median of
/// the last 30 gaps known by then (past only). Keys and order are `lower`'s.
pub fn shift(lower: &LowerRows, chart: &LowerRows, sec: i64, isec: i64) -> IndexMap<i64, f64> {
    let mut chart_t: Vec<i64> = chart.keys().copied().collect();
    chart_t.sort_unstable();
    let mut keys = Vec::new();
    let mut gaps = Vec::new();
    for t in chart_t {
        let k = t + sec - isec;
        if let Some(l) = lower.get(&k) {
            keys.push(k);
            gaps.push(chart[&t].close - l.close);
        }
    }
    if keys.is_empty() {
        return lower.keys().map(|t| (*t, 0.0)).collect();
    }
    let mut meds = Vec::with_capacity(gaps.len());
    let mut win: Vec<f64> = Vec::new();
    for g in gaps {
        win.push(g);
        if win.len() > 30 {
            win.remove(0);
        }
        let mut s = win.clone();
        py_sort(&mut s);
        meds.push(s[s.len() / 2]);
    }
    lower
        .keys()
        .map(|t| {
            let i = keys.partition_point(|k| k <= t) as i64 - 1;
            (*t, meds[i.max(0) as usize])
        })
        .collect()
}

// ---------------------------------------------------------------- delta

/// `delta()`'s start: the newest `MAX_ROWS` lower bars, whole chart bars only (not on D1).
pub fn delta_window(tf: Tf, from_s: i64, to_s: i64) -> Span {
    let (sec, isec) = (tf.secs(), intrabar(tf).secs());
    let mut a = from_s.max(to_s - MAX_ROWS * isec);
    if a.rem_euclid(sec) != 0 && tf != Tf::D1 {
        a += sec - a.rem_euclid(sec);
    }
    Span {
        a,
        b: to_s + sec - isec,
    }
}

/// Delta per chart bar from the lower bars (sorted by time), TradingView's CVD candle rules.
pub fn delta(tf: Tf, lower: &LowerRows, r: &Resolved) -> DeltaReply {
    let mut ts: Vec<i64> = lower.keys().copied().collect();
    ts.sort_unstable();
    let mut rows: Vec<DeltaRow> = Vec::new();
    for t in ts {
        let bt = bucket(tf, t);
        let v = lower[&t];
        if rows.last().is_none_or(|c| c.t != bt) {
            rows.push(DeltaRow {
                t: bt,
                buy: 0.0,
                sell: 0.0,
                delta: 0.0,
                hi: 0.0,
                lo: 0.0,
                n: 0,
            });
        }
        let cur = rows.last_mut().expect("pushed above");
        cur.buy += v.buy;
        cur.sell += v.sell;
        cur.delta += v.buy - v.sell;
        cur.hi = py_max(cur.hi, cur.delta);
        cur.lo = py_min(cur.lo, cur.delta);
        cur.n += 1;
    }
    for c in &mut rows {
        c.buy = py_round(c.buy, 6);
        c.sell = py_round(c.sell, 6);
        c.delta = py_round(c.delta, 6);
        c.hi = py_round(c.hi, 6);
        c.lo = py_round(c.lo, 6);
    }
    DeltaReply {
        rows,
        source: r.src,
        real_symbol: r.real.clone(),
        intrabar: intrabar(tf),
        unit: unit(r.src),
        method: match r.src {
            FlowSource::Real => METHOD_REAL,
            FlowSource::Estimated => METHOD_EST,
        }
        .to_string(),
    }
}

// ---------------------------------------------------------------- session volume profile

/// `mode` as Python reads it: `"day"`, `"week"`, anything else is sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionMode {
    Sessions,
    Day,
    Week,
}

impl SessionMode {
    pub fn from_param(mode: &str) -> SessionMode {
        match mode {
            "day" => SessionMode::Day,
            "week" => SessionMode::Week,
            _ => SessionMode::Sessions,
        }
    }
}

/// `(session start, name)` in UTC seconds: Asia 22:00 to 07:00, London 07:00 to 13:00, New York
/// 13:00 to 22:00; one per UTC day; weeks start Monday 00:00 UTC.
pub fn session_of(t: i64, mode: SessionMode) -> (i64, &'static str) {
    let day = t - t.rem_euclid(86_400);
    match mode {
        SessionMode::Day => return (day, "Day"),
        SessionMode::Week => {
            return (
                day - (day.div_euclid(86_400) + 3).rem_euclid(7) * 86_400,
                "Week",
            )
        }
        SessionMode::Sessions => {}
    }
    let h = t.rem_euclid(86_400).div_euclid(3600);
    if h >= 22 {
        (day + 22 * 3600, "Asia")
    } else if h < 7 {
        (day - 2 * 3600, "Asia")
    } else if h < 13 {
        (day + 7 * 3600, "London")
    } else {
        (day + 13 * 3600, "New York")
    }
}

/// The query options of a profile request, after Python's `int()`/`float()`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileOpts {
    /// The raw `mode` string (it is echoed in the reply after the H4 and D1 rewrite).
    pub mode: String,
    pub rows: i64,
    pub row_size: f64,
    pub va_pct: f64,
    pub developing: bool,
}

/// What `profile()` loads: the lower bars span, and for a real source the chart's own candles at
/// the chart timeframe (to shift Binance prices onto the chart's price).
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileWindow {
    pub tf: Tf,
    pub itf: Tf,
    /// The mode after the rewrite (H4 sessions become days, D1 sessions or days become weeks).
    pub mode: String,
    pub lower: Span,
    pub chart: Option<Span>,
}

/// `profile()`'s windows. The mode rewrite reads the tf; whole sessions only.
pub fn profile_window(
    tf: Tf,
    mode: &str,
    src: FlowSource,
    from_s: i64,
    to_s: i64,
) -> ProfileWindow {
    let mode = match (tf, mode) {
        (Tf::H4, "sessions") => "day",
        (Tf::D1, "sessions" | "day") => "week",
        (_, m) => m,
    }
    .to_string();
    let sm = SessionMode::from_param(&mode);
    let (sec, itf) = (tf.secs(), intrabar(tf));
    let isec = itf.secs();
    let mut start = from_s.max(to_s - MAX_ROWS * isec);
    let s0 = session_of(start, sm).0;
    if s0 < start {
        let mut t = start - start.rem_euclid(3600);
        while session_of(t, sm).0 == s0 {
            t += 3600;
        }
        start = t;
    }
    let lower = Span {
        a: start,
        b: to_s + sec - isec,
    };
    let chart = (src == FlowSource::Real).then_some(Span {
        a: start - 30 * sec,
        b: to_s,
    });
    ProfileWindow {
        tf,
        itf,
        mode,
        lower,
        chart,
    }
}

/// `_histogram` result: the lowest row's price, the row height, buy and sell per row.
#[derive(Clone, Debug, PartialEq)]
pub struct Histogram {
    pub lo: f64,
    pub step: f64,
    pub buy: Vec<f64>,
    pub sell: Vec<f64>,
}

/// `_histogram`: each bar's buy and sell spread evenly over the rows its high-low range covers.
/// `bars` are `(buy, sell, high, low)`. Errors where Python raises.
pub fn histogram(
    bars: &[(f64, f64, f64, f64)],
    n_rows: i64,
    row_size: f64,
) -> Result<Histogram, OfError> {
    let mut lo = bars[0].3;
    let mut hi = bars[0].2;
    for b in &bars[1..] {
        lo = py_min(lo, b.3);
        hi = py_max(hi, b.2);
    }
    let step = if row_size > 0.0 {
        row_size
    } else {
        if n_rows == 0 {
            return Err(OfError::new(
                PyClass::ZeroDivisionError,
                "float division by zero",
            ));
        }
        py_max((hi - lo) / n_rows as f64, 1e-12)
    };
    if row_size > 0.0 {
        lo -= py_mod(lo, step);
    }
    let n = if row_size > 0.0 {
        1_i64.max(((hi - lo) / step) as i64 + 1)
    } else {
        n_rows
    };
    if n > MAX_PROFILE_ROWS {
        return Err(too_many_rows());
    }
    if n <= 0 {
        return Err(OfError::new(PyClass::IndexError, "list index out of range"));
    }
    let n = n as usize;
    let (mut buy, mut sell) = (vec![0.0; n], vec![0.0; n]);
    let idx = |x: f64| (n as i64 - 1).min(0_i64.max(((x - lo) / step) as i64)) as usize;
    for &(b, s, h, l) in bars {
        let (i0, i1) = (idx(l), idx(h));
        if i1 == i0 || h <= l {
            buy[i0] += b;
            sell[i0] += s;
            continue;
        }
        for i in i0..=i1 {
            let r_lo = lo + i as f64 * step;
            let r_hi = lo + (i + 1) as f64 * step;
            let w = (py_min(h, r_hi) - py_max(l, r_lo)) / (h - l);
            if w > 0.0 {
                buy[i] += b * w;
                sell[i] += s * w;
            }
        }
    }
    Ok(Histogram {
        lo,
        step,
        buy,
        sell,
    })
}

/// `_levels` with `_value_area`: the POC row (first maximum), its price, VAH and VAL.
pub fn levels(h: &Histogram, va_pct: f64) -> (usize, f64, f64, f64) {
    let tot: Vec<f64> = h.buy.iter().zip(&h.sell).map(|(x, y)| x + y).collect();
    let mut poc = 0;
    for (i, v) in tot.iter().enumerate() {
        if *v > tot[poc] {
            poc = i;
        }
    }
    let total = py_sum(tot.iter().copied());
    let (mut lo, mut hi) = (poc, poc);
    let mut inside = tot[poc];
    let last = tot.len() - 1;
    while total != 0.0 && inside < va_pct * total && (lo > 0 || hi < last) {
        let up = if hi < last { tot[hi + 1] } else { -1.0 };
        let dn = if lo > 0 { tot[lo - 1] } else { -1.0 };
        if up >= dn {
            hi += 1;
            inside += up;
        } else {
            lo -= 1;
            inside += dn;
        }
    }
    (
        poc,
        h.lo + (poc as f64 + 0.5) * h.step,
        h.lo + (hi + 1) as f64 * h.step,
        h.lo + lo as f64 * h.step,
    )
}

/// The profile per session, on the chart's prices. For a real source `chart` holds the chart's
/// own candles at the chart timeframe and Binance prices are shifted onto them.
pub fn profile(
    w: &ProfileWindow,
    opts: &ProfileOpts,
    r: &Resolved,
    lower: &LowerRows,
    chart: Option<&LowerRows>,
) -> Result<ProfileReply, OfError> {
    let (sec, isec) = (w.tf.secs(), w.itf.secs());
    let sm = SessionMode::from_param(&w.mode);
    let lower: LowerRows = match (r.src, chart) {
        (FlowSource::Real, Some(chart)) => {
            let g_at = shift(lower, chart, sec, isec);
            lower
                .iter()
                .filter_map(|(t, v)| {
                    g_at.get(t).map(|g| {
                        (
                            *t,
                            Lower {
                                high: v.high + g,
                                low: v.low + g,
                                close: v.close + g,
                                ..*v
                            },
                        )
                    })
                })
                .collect()
        }
        _ => lower.clone(),
    };
    let mut ts: Vec<i64> = lower.keys().copied().collect();
    ts.sort_unstable();
    let mut groups: BTreeMap<(i64, &'static str), Vec<i64>> = BTreeMap::new();
    for t in ts {
        groups.entry(session_of(t, sm)).or_default().push(t);
    }
    let mut sessions = Vec::with_capacity(groups.len());
    for ((s0, name), ts) in groups {
        let bars: Vec<(f64, f64, f64, f64)> = ts
            .iter()
            .map(|t| {
                let v = lower[t];
                (v.buy, v.sell, v.high, v.low)
            })
            .collect();
        let h = histogram(&bars, opts.rows, opts.row_size)?;
        let (poc_i, poc, vah, val) = levels(&h, opts.va_pct);
        let mut dev = Vec::new();
        if opts.developing {
            for (k, t) in ts.iter().enumerate() {
                let closes_bar = k == ts.len() - 1
                    || (ts[k + 1] - ts[k + 1].rem_euclid(sec)) != (t - t.rem_euclid(sec));
                if closes_bar {
                    let h2 = histogram(&bars[..=k], opts.rows, opts.row_size)?;
                    let (_, p2, v_hi, v_lo) = levels(&h2, opts.va_pct);
                    dev.push(DevPoint(
                        bucket(w.tf, *t),
                        py_round(p2, 6),
                        py_round(v_hi, 6),
                        py_round(v_lo, 6),
                    ));
                }
            }
        }
        let (sb, ss) = (
            py_sum(h.buy.iter().copied()),
            py_sum(h.sell.iter().copied()),
        );
        sessions.push(Session {
            start: s0,
            name: name.to_string(),
            first: ts[0],
            last: ts[ts.len() - 1] + isec,
            lo: h.lo,
            step: h.step,
            buy: h.buy.iter().map(|x| py_round(*x, 4)).collect(),
            sell: h.sell.iter().map(|x| py_round(*x, 4)).collect(),
            poc_i: poc_i as i64,
            poc,
            vah,
            val,
            total: py_round(sb + ss, 4),
            delta: py_round(sb - ss, 4),
            dev,
        });
    }
    let method = match r.src {
        FlowSource::Real => METHOD_REAL,
        FlowSource::Estimated => METHOD_EST,
    };
    Ok(ProfileReply {
        sessions,
        source: r.src,
        real_symbol: r.real.clone(),
        intrabar: w.itf,
        unit: unit(r.src),
        mode: w.mode.clone(),
        va_pct: opts.va_pct,
        method: format!("{method}; each lower bar's volume spread evenly over its high-low range"),
        shift: (r.src == FlowSource::Real)
            .then(|| "Binance prices shifted onto the chart price (median gap of the last 30 chart candles)".to_string()),
    })
}

// ---------------------------------------------------------------- footprint

/// Auto row size: about 7 rows in a typical candle (`nice_step(median(h - l) / 7)`), else 1.0.
pub fn auto_row_size(chart_bars: &[(i64, f64, f64)]) -> Num {
    let mut rng: Vec<f64> = chart_bars
        .iter()
        .filter(|(_, h, l)| h > l)
        .map(|(_, h, l)| h - l)
        .collect();
    if rng.is_empty() {
        return Num::Float(1.0);
    }
    py_sort(&mut rng);
    nice_step(rng[rng.len() / 2] / 7.0)
}

/// The spans a footprint loads: real 1-minute rows and, for a real source, the chart's own candles.
pub fn footprint_spans(tf: Tf, from_s: i64, to_s: i64) -> (Span, Span, (i64, i64)) {
    let sec = tf.secs();
    let m1 = Span {
        a: from_s,
        b: to_s + sec - 60,
    };
    let chart = Span {
        a: from_s - 30 * sec,
        b: to_s,
    };
    let trades = (from_s * 1000, (to_s + sec) * 1000 - 1);
    (m1, chart, trades)
}

/// What a footprint is computed from.
pub enum FootprintData<'a> {
    /// Every Binance trade: 1-minute klines (for the shift and the estimate fill), the shift per
    /// minute, and the trade window.
    Real {
        m1: &'a LowerRows,
        g_at: &'a IndexMap<i64, f64>,
        window: &'a TradeWindow,
    },
    /// The chart's own 1-minute candles.
    Estimated { m1: &'a LowerRows },
}

fn row_of(x: f64, rs: f64) -> i64 {
    (x / rs + 1e-9).floor() as i64
}

/// The request side of a footprint: chart timeframe, range, row size (already resolved: manual
/// or `auto_row_size`) and whether minutes still loading are estimated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootprintArgs {
    pub tf: Tf,
    pub from_s: i64,
    pub to_s: i64,
    pub row_size: Num,
    pub fill: bool,
}

/// Buy and sell at each price row inside every chart candle in `[from_s, to_s]`, on the chart's
/// prices. `now_s` is `time.time()` (the forming minute counts as whole once loaded up to now).
pub fn footprint(
    args: &FootprintArgs,
    data: FootprintData<'_>,
    now_s: f64,
    r: &Resolved,
) -> Result<FootprintReply, OfError> {
    let FootprintArgs {
        tf,
        from_s,
        to_s,
        row_size,
        fill,
    } = *args;
    let rs = row_size.as_f64();
    let sec = tf.secs();
    let mut candles: BTreeMap<i64, BTreeMap<i64, [f64; 2]>> = BTreeMap::new();
    let mut est: BTreeSet<i64> = BTreeSet::new();
    let (mut pending, mut cov): (bool, &[(i64, i64)]) = (false, &[]);
    let spread = |candles: &mut BTreeMap<i64, BTreeMap<i64, [f64; 2]>>,
                  bt: i64,
                  lo: f64,
                  hi: f64,
                  buy: f64,
                  sell: f64|
     -> Result<(), OfError> {
        let (r0, r1) = (row_of(lo, rs), row_of(hi, rs));
        if r1 - r0 + 1 > MAX_CANDLE_ROWS {
            return Err(too_many_rows());
        }
        let n = (r1 - r0 + 1) as f64;
        let c = candles.entry(bt).or_default();
        for row in r0..=r1 {
            let x = c.entry(row).or_insert([0.0, 0.0]);
            x[0] += buy / n;
            x[1] += sell / n;
        }
        Ok(())
    };
    match data {
        FootprintData::Real { m1, g_at, window } => {
            cov = &window.covered;
            pending = window.pending;
            let live = now_s * 1000.0 - 3.0 * POLL_S * 1000.0;
            let full: std::collections::HashSet<i64> = m1
                .keys()
                .copied()
                .filter(|m| {
                    cov.iter().any(|&(a, b)| {
                        a <= m * 1000 && (b >= m * 1000 + 59_999 || b as f64 >= live)
                    })
                })
                .collect();
            let tw = &window.trades;
            let keep: Vec<usize> = (0..tw.len())
                .filter(|&i| full.contains(&(tw.t_ms[i].div_euclid(60_000) * 60)))
                .collect();
            if !keep.is_empty() {
                let mut gk: Vec<i64> = g_at.keys().copied().collect();
                gk.sort_unstable();
                let gv: Vec<f64> = gk.iter().map(|k| g_at[k]).collect();
                let rows: Vec<i64> = keep
                    .iter()
                    .map(|&i| {
                        let min = tw.t_ms[i].div_euclid(60_000) * 60;
                        let g = if gk.is_empty() {
                            0.0
                        } else {
                            let j = (gk.partition_point(|k| *k <= min) as i64 - 1)
                                .clamp(0, gv.len() as i64 - 1);
                            gv[j as usize]
                        };
                        row_of(tw.price[i] + g, rs)
                    })
                    .collect();
                let base = *rows.iter().min().expect("non-empty");
                for (side, col) in [(1_i8, 0_usize), (-1, 1)] {
                    let mut sums: BTreeMap<i64, f64> = BTreeMap::new();
                    for (n, &i) in keep.iter().enumerate() {
                        if tw.side[i] != side {
                            continue;
                        }
                        let bt = bucket(tf, tw.t_ms[i].div_euclid(1000));
                        *sums
                            .entry(bt * 100_000_000 + (rows[n] - base))
                            .or_insert(0.0) += tw.qty[i];
                    }
                    for (kx, v) in sums {
                        let c = candles.entry(kx.div_euclid(100_000_000)).or_default();
                        c.entry(kx.rem_euclid(100_000_000) + base)
                            .or_insert([0.0, 0.0])[col] += v;
                    }
                }
            }
            if fill {
                for (m, v) in m1 {
                    let bt = bucket(tf, *m);
                    if full.contains(m) || !(from_s <= bt && bt <= to_s) {
                        continue;
                    }
                    let gg = g_at.get(m).copied().unwrap_or(0.0);
                    spread(&mut candles, bt, v.low + gg, v.high + gg, v.buy, v.sell)?;
                    est.insert(bt);
                }
            }
        }
        FootprintData::Estimated { m1 } => {
            for (m, v) in m1 {
                let bt = bucket(tf, *m);
                if from_s <= bt && bt <= to_s {
                    spread(&mut candles, bt, v.low, v.high, v.buy, v.sell)?;
                }
            }
        }
    }
    let too_wide =
        |rows: &BTreeMap<i64, [f64; 2]>| match (rows.keys().next(), rows.keys().next_back()) {
            (Some(lo), Some(hi)) => hi - lo + 1 > MAX_CANDLE_ROWS,
            _ => false,
        };
    if candles.values().any(too_wide) {
        return Err(too_many_rows());
    }
    let out: Vec<Candle> = candles
        .iter()
        .map(|(bt, rows)| {
            let lo = *rows.keys().next().expect("a candle has rows");
            let hi = *rows.keys().next_back().expect("a candle has rows");
            let cell = |r: i64, col: usize| {
                rows.get(&r)
                    .map_or(Vol::Gap, |x| Vol::V(py_round(x[col], 4)))
            };
            Candle {
                t: *bt,
                r0: lo,
                buy: (lo..=hi).map(|r| cell(r, 0)).collect(),
                sell: (lo..=hi).map(|r| cell(r, 1)).collect(),
                est: est.contains(bt),
            }
        })
        .collect();
    let first_cov = cov
        .iter()
        .filter(|(_, b)| *b >= from_s * 1000)
        .map(|(a, _)| *a)
        .min()
        .unwrap_or((to_s + sec) * 1000);
    let method = match r.src {
        FlowSource::Real if !est.is_empty() => "every Binance futures trade, shifted onto the chart price; candles marked est use Binance 1-minute buy/sell totals until their trades load",
        FlowSource::Real => "every Binance futures trade, shifted onto the chart price",
        FlowSource::Estimated => "chart 1-minute candles spread over their range (coarse estimate)",
    };
    Ok(FootprintReply {
        candles: out,
        source: r.src,
        real_symbol: r.real.clone(),
        row_size,
        unit: unit(r.src),
        covered_from: first_cov.max(from_s * 1000).div_euclid(1000),
        pending: pending && !est.is_empty(),
        est_candles: est.len() as i64,
        method: method.to_string(),
    })
}

/// `_union`: sorted, touching or overlapping ranges merged.
pub fn union(iv: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut v = iv.to_vec();
    v.sort_unstable();
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 + 1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// `_covers`: `[a, b]` lies inside one unioned range (an empty range is covered).
pub fn covers(iv: &[(i64, i64)], a: i64, b: i64) -> bool {
    b < a || union(iv).iter().any(|&(x, y)| x <= a && y >= b)
}

#[cfg(test)]
pub(crate) mod numerics {
    use super::*;
    use serde_json::Value;

    fn table() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/orderflow/py_numeric.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("py_numeric.json"))
            .expect("json")
    }

    fn f(v: &Value) -> f64 {
        v.as_f64().expect("number")
    }

    fn same(a: f64, b: f64) -> bool {
        a.to_bits() == b.to_bits()
    }

    #[test]
    fn round_matches_python() {
        let t = table();
        let mut bad = Vec::new();
        for r in t["round"].as_array().unwrap() {
            let (x, nd, want) = (f(&r[0]), r[1].as_u64().unwrap() as usize, f(&r[2]));
            if !same(py_round(x, nd), want) {
                bad.push(format!(
                    "round({x:?}, {nd}) = {:?}, Python {want:?}",
                    py_round(x, nd)
                ));
            }
        }
        assert!(
            bad.is_empty(),
            "{} of {} differ: {:?}",
            bad.len(),
            t["round"].as_array().unwrap().len(),
            &bad[..bad.len().min(5)]
        );
    }

    #[test]
    fn sum_matches_python_and_a_fold_does_not() {
        let t = table();
        let mut fold_differs = 0;
        for r in t["sum"].as_array().unwrap() {
            let xs: Vec<f64> = r[0].as_array().unwrap().iter().map(f).collect();
            let want = f(&r[1]);
            assert!(
                same(py_sum(xs.iter().copied()), want),
                "sum({xs:?}) = {:?}, Python {want:?}",
                py_sum(xs.iter().copied())
            );
            let fold = xs.iter().fold(0.0, |a, b| a + b);
            fold_differs += usize::from(!same(fold, want));
        }
        assert!(
            fold_differs > 0,
            "the table must contain lists where a plain fold differs"
        );
    }

    #[test]
    fn mod_max_min_match_python() {
        let t = table();
        for r in t["mod"].as_array().unwrap() {
            let (a, b, want) = (f(&r[0]), f(&r[1]), f(&r[2]));
            assert!(
                same(py_mod(a, b), want),
                "{a:?} % {b:?} = {:?}, Python {want:?}",
                py_mod(a, b)
            );
        }
        for r in t["maxmin"].as_array().unwrap() {
            let (a, b) = (f(&r[0]), f(&r[1]));
            assert!(same(py_max(a, b), f(&r[2])), "max({a:?}, {b:?})");
            assert!(same(py_min(a, b), f(&r[3])), "min({a:?}, {b:?})");
        }
        let mut v = vec![0.0, -0.0, 1.0, -0.0, 0.0];
        py_sort(&mut v);
        assert_eq!(
            v.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            [0.0f64, -0.0, -0.0, 0.0, 1.0]
                .iter()
                .map(|x| x.to_bits())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn nice_step_value_and_kind_match_python() {
        let t = table();
        for r in t["nice_step"].as_array().unwrap() {
            let x = f(&r[0]);
            let got = nice_step(x);
            match (r[1].as_str().unwrap(), got) {
                ("int", Num::Int(i)) => assert_eq!(Some(i), r[2].as_i64(), "nice_step({x:?})"),
                ("float", Num::Float(v)) => assert!(
                    same(v, f(&r[2])),
                    "nice_step({x:?}) = {v:?}, Python {}",
                    r[2]
                ),
                (kind, got) => panic!("nice_step({x:?}) = {got:?}, Python {kind} {}", r[2]),
            }
        }
    }

    #[test]
    fn int_and_float_args_match_python() {
        let t = table();
        for r in t["int"].as_array().unwrap() {
            let s = r["s"].as_str().unwrap();
            match (py_int(s), r.get("ok")) {
                (Ok(v), Some(want)) => assert_eq!(Some(v), want.as_i64(), "int({s:?})"),
                (Err(e), None) => assert_eq!(e.msg, r["err"].as_str().unwrap(), "int({s:?})"),
                (got, _) => panic!("int({s:?}) = {got:?}, Python {r}"),
            }
        }
        for r in t["float"].as_array().unwrap() {
            let s = r["s"].as_str().unwrap();
            match (py_float(s), r.get("ok")) {
                (Ok(v), Some(Value::String(w))) => assert_eq!(
                    format!("{v}").to_lowercase(),
                    w.to_lowercase(),
                    "float({s:?})"
                ),
                (Ok(v), Some(want)) => assert!(same(v, f(want)), "float({s:?})"),
                (Err(e), None) => assert_eq!(e.msg, r["err"].as_str().unwrap(), "float({s:?})"),
                (got, _) => panic!("float({s:?}) = {got:?}, Python {r}"),
            }
        }
        assert_eq!(py_repr("a'b"), "\"a'b\"");
        assert_eq!(py_repr("x"), "'x'");
    }

    #[test]
    fn error_classes_print() {
        let e = OfError::new(PyClass::ZeroDivisionError, "float division by zero");
        assert_eq!(e.to_string(), "ZeroDivisionError: float division by zero");
    }

    // ---------------------------------------------------------------- helpers vs Python (py_units.json)

    fn units() -> Value {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/orderflow/py_units.json"
        );
        serde_json::from_str(&std::fs::read_to_string(path).expect("py_units.json")).expect("json")
    }

    fn rows_from(v: &Value) -> LowerRows {
        v.as_array()
            .unwrap()
            .iter()
            .map(|r| {
                let t = r[1].as_array().unwrap();
                (
                    r[0].as_i64().unwrap(),
                    Lower {
                        buy: f(&t[0]),
                        sell: f(&t[1]),
                        sign: t[2].as_i64().unwrap() as i8,
                        high: f(&t[3]),
                        low: f(&t[4]),
                        close: f(&t[5]),
                    },
                )
            })
            .collect()
    }

    fn assert_rows_eq(got: &LowerRows, want: &LowerRows, what: &str) {
        let g: Vec<_> = got
            .iter()
            .map(|(t, v)| {
                (
                    *t,
                    v.buy.to_bits(),
                    v.sell.to_bits(),
                    v.sign,
                    v.high.to_bits(),
                    v.low.to_bits(),
                    v.close.to_bits(),
                )
            })
            .collect();
        let w: Vec<_> = want
            .iter()
            .map(|(t, v)| {
                (
                    *t,
                    v.buy.to_bits(),
                    v.sell.to_bits(),
                    v.sign,
                    v.high.to_bits(),
                    v.low.to_bits(),
                    v.close.to_bits(),
                )
            })
            .collect();
        assert_eq!(g, w, "{what}: rows or their order differ");
    }

    #[test]
    fn symbols_and_sessions_match_python() {
        let u = units();
        let fut: BTreeSet<String> = u["futures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect();
        for r in u["real_symbol"].as_array().unwrap() {
            let raw = r[0].as_str().unwrap();
            assert_eq!(
                real_symbol(raw, &fut).as_deref(),
                r[1].as_str(),
                "real_symbol({raw})"
            );
        }
        for r in u["session_of"].as_array().unwrap() {
            let (t, mode) = (r[0].as_i64().unwrap(), r[1].as_str().unwrap());
            let (s0, name) = session_of(t, SessionMode::from_param(mode));
            assert_eq!(
                (s0, name),
                (r[2][0].as_i64().unwrap(), r[2][1].as_str().unwrap()),
                "session_of({t}, {mode})"
            );
        }
    }

    #[test]
    fn estimated_rows_and_need_volume_match_python() {
        for e in units()["estimated"].as_array().unwrap() {
            let chunks: Vec<Vec<Bar>> = serde_json::from_value(e["chunks"].clone()).unwrap();
            let span = Span {
                a: e["a"].as_i64().unwrap(),
                b: e["b"].as_i64().unwrap(),
            };
            let got = estimated_rows(&chunks, span);
            assert_rows_eq(&got, &rows_from(&e["rows"]), e["name"].as_str().unwrap());
            match (need_volume(&got), e["need_volume"].get("err")) {
                (Ok(()), None) => {}
                (Err(err), Some(want)) => assert_eq!(err.to_string(), want.as_str().unwrap()),
                (got, want) => panic!("need_volume {got:?}, Python {want:?}"),
            }
        }
    }

    #[test]
    fn shift_matches_python() {
        for c in units()["shift"].as_array().unwrap() {
            let got = shift(
                &rows_from(&c["lower"]),
                &rows_from(&c["chart"]),
                c["sec"].as_i64().unwrap(),
                c["isec"].as_i64().unwrap(),
            );
            let want: Vec<(i64, u64)> = c["out"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| (r[0].as_i64().unwrap(), f(&r[1]).to_bits()))
                .collect();
            let got: Vec<(i64, u64)> = got.iter().map(|(t, g)| (*t, g.to_bits())).collect();
            assert_eq!(got, want, "shift {}", c["name"]);
        }
    }

    #[test]
    fn histogram_and_levels_match_python() {
        for c in units()["histogram"].as_array().unwrap() {
            let bars: Vec<(f64, f64, f64, f64)> = c["bars"]
                .as_array()
                .unwrap()
                .iter()
                .map(|b| (f(&b[0]), f(&b[1]), f(&b[2]), f(&b[3])))
                .collect();
            let got = histogram(&bars, c["rows"].as_i64().unwrap(), f(&c["row_size"]));
            match (got, &c["result"]) {
                (Ok(h), r) if r.get("ok").is_some() => {
                    let ok = &r["ok"];
                    assert!(
                        same(h.lo, f(&ok[0])) && same(h.step, f(&ok[1])),
                        "{} lo/step",
                        c["name"]
                    );
                    let bits = |v: &Value| {
                        v.as_array()
                            .unwrap()
                            .iter()
                            .map(|x| f(x).to_bits())
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(
                        h.buy.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        bits(&ok[2]),
                        "{} buy",
                        c["name"]
                    );
                    assert_eq!(
                        h.sell.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        bits(&ok[3]),
                        "{} sell",
                        c["name"]
                    );
                    let (poc_i, poc, vah, val) = levels(&h, f(&c["va"]));
                    let l = &ok[4];
                    assert_eq!(poc_i as i64, l[0].as_i64().unwrap(), "{} poc_i", c["name"]);
                    assert!(
                        same(poc, f(&l[1])) && same(vah, f(&l[2])) && same(val, f(&l[3])),
                        "{} levels",
                        c["name"]
                    );
                }
                (Err(e), r) => assert_eq!(
                    Some(e.to_string().as_str()),
                    r["err"].as_str(),
                    "{}",
                    c["name"]
                ),
                (Ok(h), r) => panic!("{}: got {h:?}, Python {r}", c["name"]),
            }
        }
    }

    #[test]
    fn union_covers_and_rows_cache_rules_match_python() {
        let u = units();
        for c in u["union"].as_array().unwrap() {
            let iv: Vec<(i64, i64)> = c[0]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_i64().unwrap(), p[1].as_i64().unwrap()))
                .collect();
            let want: Vec<(i64, i64)> = c[1]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_i64().unwrap(), p[1].as_i64().unwrap()))
                .collect();
            assert_eq!(union(&iv), want);
            for q in c[2].as_array().unwrap() {
                assert_eq!(
                    covers(&iv, q[0].as_i64().unwrap(), q[1].as_i64().unwrap()),
                    q[2].as_bool().unwrap(),
                    "covers {iv:?} {q}"
                );
            }
        }
        // the fake fetch of the Python run: one row per minute from a (aligned) to b
        let fetch = |s: Span| -> LowerRows {
            let first = s.a - s.a.rem_euclid(60);
            (first..=s.b)
                .step_by(60)
                .filter(|t| *t >= s.a)
                .map(|t| {
                    (
                        t,
                        Lower {
                            buy: 1.0,
                            sell: 0.5,
                            sign: 0,
                            high: 2.0,
                            low: 1.0,
                            close: 1.5 + t as f64 / 1e6,
                        },
                    )
                })
                .collect()
        };
        let mut have = LowerRows::new();
        for c in u["rows_sequence"].as_array().unwrap() {
            let span = Span {
                a: c["a"].as_i64().unwrap(),
                b: c["b"].as_i64().unwrap(),
            };
            let need = rows_need(&have, 60, span.a, span.b);
            let want: Vec<(i64, i64)> = c["fetched"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (p[0].as_i64().unwrap(), p[1].as_i64().unwrap()))
                .collect();
            assert_eq!(
                need.iter().map(|s| (s.a, s.b)).collect::<Vec<_>>(),
                want,
                "plan for {span:?}"
            );
            for s in need {
                rows_merge(&mut have, fetch(s));
            }
            let keys: Vec<i64> = rows_window(&have, span).keys().copied().collect();
            let want_keys: Vec<i64> = c["keys"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_i64().unwrap())
                .collect();
            assert_eq!(keys, want_keys, "insertion order after {span:?}");
        }
    }
}
