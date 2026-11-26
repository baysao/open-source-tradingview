//! The domain core: the oracle's arithmetic, ported exactly.
//!
//! Every function here answers with the same numbers the Python bridge
//! (`bridge/mt5_bridge.py`) answers, because 08-03's contract suite diffs the two byte for byte.
//! The MCP surface is the five tool names the oracle calls; nothing else reaches the terminal.
//!
//! Units: every time is UTC seconds unless the name says `_ms`.

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value};

use crate::mcp::{Mcp, McpError};

/// The five tool names the oracle calls. This is the whole MCP surface of the bridge; the const
/// exists for the parity grep and the test that pins the count, and the call sites spell the names.
#[cfg_attr(not(test), allow(dead_code))]
pub const TOOLS: [&str; 5] = [
    "get_time_information",
    "get_chart_history",
    "get_chart_ticks_history",
    "get_marketwatch_symbols",
    "get_trading_account_info",
];

/// The category-to-path map (the oracle's `CATEGORY_PATH`, keys lowercased).
const CATEGORY_PATH: [(&str, &str); 8] = [
    ("majors", "Forex"),
    ("minors", "Forex"),
    ("exotic", "Forex"),
    ("crypto", "Crypto"),
    ("metals", "Metals"),
    ("indices", "Indices"),
    ("energies", "Energies"),
    ("stocks", "Stocks"),
];

/// A terminal that answers MCP tool calls. [`Mcp`] in production, an in-memory fake under test
/// (L-016: no sockets in the domain tests).
pub trait Terminal: Send + Sync {
    fn call(&self, name: &str, args: Value)
        -> impl Future<Output = Result<Value, McpError>> + Send;
}

impl Terminal for Mcp {
    fn call(
        &self,
        name: &str,
        args: Value,
    ) -> impl Future<Output = Result<Value, McpError>> + Send {
        Mcp::call(self, name, args)
    }
}

/// The three sessions the oracle keeps: a long history read never delays the live candle.
pub struct Terminals<M: Terminal> {
    pub live: Arc<M>,
    pub hist: Arc<M>,
    pub misc: Arc<M>,
}

impl<M: Terminal> Terminals<M> {
    pub fn new(live: Arc<M>, hist: Arc<M>, misc: Arc<M>) -> Self {
        Self { live, hist, misc }
    }
}

/// A timeframe, exactly the seven the oracle accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    pub fn sec(self) -> i64 {
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

/// The oracle's `tf_sec`: an unknown name raises `unknown timeframe <name>`, and that text reaches
/// the caller verbatim (parity contract rule 7).
impl TryFrom<&str> for Tf {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "M1" => Ok(Tf::M1),
            "M5" => Ok(Tf::M5),
            "M15" => Ok(Tf::M15),
            "M30" => Ok(Tf::M30),
            "H1" => Ok(Tf::H1),
            "H4" => Ok(Tf::H4),
            "D1" => Ok(Tf::D1),
            other => Err(format!("unknown timeframe {other}")),
        }
    }
}

impl fmt::Display for Tf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One candle, in the oracle's `to_bar` key order (the `/bars` body is diffed as raw bytes).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bar {
    pub time: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

/// The `/wait` change key: the oracle's `key()`, in its order (time, close, high, low, volume).
pub fn bar_key(b: Option<&Bar>) -> String {
    match b {
        None => String::new(),
        Some(b) => format!(
            "{},{},{},{},{}",
            b.time,
            py_str(b.close),
            py_str(b.high),
            py_str(b.low),
            py_str(b.volume)
        ),
    }
}

/// Python's `str(float)`: the shortest round-trip digits, fixed notation for decimal exponents
/// -4..16 and `e±NN` outside it. Rust's `{}` differs (`1829`, `10000000000000000`), and the
/// `/wait` key is diffed byte for byte, so the shapes are rebuilt here.
pub fn py_str(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    let neg = x.is_sign_negative();
    let sci = format!("{:e}", x.abs());
    let (mant, exp) = sci.split_once('e').expect("{:e} always has an exponent");
    let exp: i32 = exp
        .parse()
        .expect("{:e} always ends in an integer exponent");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    if (-4..16).contains(&exp) {
        if exp >= 0 {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                s.push_str(&digits);
                for _ in digits.len()..int_len {
                    s.push('0');
                }
                s.push_str(".0");
            } else {
                s.push_str(&digits[..int_len]);
                s.push('.');
                s.push_str(&digits[int_len..]);
            }
        } else {
            s.push_str("0.");
            for _ in 0..(-exp - 1) {
                s.push('0');
            }
            s.push_str(&digits);
        }
    } else {
        s.push_str(&digits[..1]);
        if digits.len() > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push('e');
        s.push(if exp < 0 { '-' } else { '+' });
        let abs = exp.abs();
        if abs < 10 {
            s.push('0');
        }
        s.push_str(&abs.to_string());
    }
    s
}

/// `PYTHON`'s `iso()`: `YYYY-MM-DDTHH:MM:SS`, UTC, no suffix.
pub fn iso(epoch: i64) -> String {
    use chrono::{DateTime, Datelike, Timelike};
    let dt = DateTime::from_timestamp(epoch, 0).unwrap_or_default();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        dt.year(),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second()
    )
}

/// The oracle's `parse_time()`: read the first 19 characters as a naive UTC timestamp.
pub fn parse_time(s: &str) -> Result<i64, McpError> {
    use chrono::{NaiveDate, TimeZone, Utc};
    let head = s
        .get(..19)
        .ok_or_else(|| McpError::Parse(format!("time shorter than 19 chars: {s}")))?;
    let num = |r: std::ops::Range<usize>| head.get(r).and_then(|t| t.parse::<i32>().ok());
    let sep = head.as_bytes().get(10).copied();
    if sep != Some(b'T') && sep != Some(b' ') {
        return Err(McpError::Parse(format!("not an ISO time: {s}")));
    }
    let (y, mo, d, h, mi, se) = (
        num(0..4),
        num(5..7),
        num(8..10),
        num(11..13),
        num(14..16),
        num(17..19),
    );
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) = (y, mo, d, h, mi, se) else {
        return Err(McpError::Parse(format!("not an ISO time: {s}")));
    };
    NaiveDate::from_ymd_opt(y, mo as u32, d as u32)
        .and_then(|day| day.and_hms_opt(h as u32, mi as u32, se as u32))
        .map(|dt| Utc.from_utc_datetime(&dt).timestamp())
        .ok_or_else(|| McpError::Parse(format!("not an ISO time: {s}")))
}

/// Server time offset state. The offset task is the only writer (08-02).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offset {
    pub offset_s: i64,
    pub offset_src: String,
    pub measured_at: i64,
}

/// The oracle's `STATE` before any measurement.
impl Default for Offset {
    fn default() -> Self {
        Self {
            offset_s: 0,
            offset_src: "default (not yet measured)".to_string(),
            measured_at: 0,
        }
    }
}

/// The measure step's output; `None` is the oracle's refusal (a market-shut server clock).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Measured {
    pub offset_s: i64,
    pub measured_at: i64,
}

/// How long `/bars` waits for the terminal to sync an idle symbol, and the pause between passes.
/// Tests shrink both (the oracle's `SYNC_CAP_S` and the loop's `sleep(0.4)`).
#[derive(Clone, Copy, Debug)]
pub struct SyncPolicy {
    pub cap: std::time::Duration,
    pub step: std::time::Duration,
}

impl Default for SyncPolicy {
    fn default() -> Self {
        Self {
            cap: std::time::Duration::from_secs(6),
            step: std::time::Duration::from_millis(400),
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `float(v)` for a JSON number or a numeric string.
fn as_f64(v: Option<&Value>, field: &str) -> Result<f64, McpError> {
    match v {
        Some(Value::Number(n)) => n
            .as_f64()
            .ok_or_else(|| McpError::Parse(format!("{field} is not a number"))),
        Some(Value::String(s)) => s
            .trim()
            .parse::<f64>()
            .map_err(|_| McpError::Parse(format!("{field} is not a number: {s}"))),
        _ => Err(McpError::Parse(format!("{field} is missing"))),
    }
}

/// The oracle's `to_bar`: shift the terminal's server time back to UTC, `tick_volume` -> volume.
fn to_bar(row: &Value) -> Result<Bar, McpError> {
    let time = row
        .get("time")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::Parse("the history row has no time".to_string()))?;
    Ok(Bar {
        time: parse_time(time)?,
        open: as_f64(row.get("open"), "open")?,
        high: as_f64(row.get("high"), "high")?,
        low: as_f64(row.get("low"), "low")?,
        close: as_f64(row.get("close"), "close")?,
        volume: as_f64(row.get("tick_volume"), "tick_volume")?,
    })
}

/// One history read: no `limit` is sent, because a limited answer also drops the candle still
/// forming (the oracle's note at `read_bars`); the trim happens on our side.
pub(crate) async fn read_bars<M: Terminal>(
    hist: &M,
    sym: &str,
    tf: Tf,
    newest_utc: i64,
    span_s: i64,
    off: i64,
) -> Result<Vec<Bar>, McpError> {
    let j = hist
        .call(
            "get_chart_history",
            json!({
                "symbol": sym,
                "period": tf.as_str(),
                "datetime_from": iso(newest_utc - span_s + off),
                "datetime_to": iso(newest_utc + off),
            }),
        )
        .await?;
    let rows = j.get("history").and_then(Value::as_array);
    let mut out = Vec::with_capacity(rows.map_or(0, Vec::len));
    for row in rows.into_iter().flatten() {
        let mut bar = to_bar(row)?;
        bar.time -= off;
        out.push(bar);
    }
    Ok(out)
}

/// The oracle's `bars()`: newest `limit` candles, resyncing an idle symbol.
///
/// The terminal syncs an idle symbol only when asked, so the first read can be old or short:
/// read again, widening the window while the answer is short, until the last candle is near the
/// newest tick and the count has stopped growing, or the policy's cap runs out.
pub async fn bars<M: Terminal>(
    hist: &M,
    sym: &str,
    tf: Tf,
    limit: i64,
    to_ms: Option<i64>,
    off: i64,
    policy: &SyncPolicy,
) -> Result<Vec<Bar>, McpError> {
    let sec = tf.sec();
    let limit = limit.clamp(1, 5000);
    let newest = match to_ms {
        Some(ms) => ms / 1000,
        None => now_secs() + 86400,
    };
    // Weekends and holidays make bar counts short; widened below while needed.
    let mut span: i64 = limit * sec * 3 / 2 + 4 * 86400;
    let end = tokio::time::Instant::now() + policy.cap;
    let mut last_n: i64 = -1;
    loop {
        let out = read_bars(hist, sym, tf, newest, span, off).await?;
        let grown = out.len() as i64 != last_n;
        last_n = out.len() as i64;
        if last_n < limit && span < 30 * 365 * 86400 {
            span *= 3;
            if tokio::time::Instant::now() <= end {
                continue;
            }
        }
        let tick = match to_ms {
            Some(_) => None,
            None => last_tick(hist, sym, off).await?,
        };
        let stale = match (tick, out.last()) {
            (Some(t), Some(b)) => t / 1000 - b.time >= sec,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if (!stale && (!grown || last_n >= limit)) || tokio::time::Instant::now() > end {
            let keep = limit as usize;
            let start = out.len().saturating_sub(keep);
            return Ok(out[start..].to_vec());
        }
        tokio::time::sleep(policy.step).await;
    }
}

/// The oracle's tick-ms slice: characters 20..23 of `time_ms` are the milliseconds; absent = 0.
/// Python's slice clamps at the string's end, so a shorter stamp yields whatever digits remain.
pub fn tick_ms(time_ms: &str, off: i64) -> Result<i64, McpError> {
    let base = parse_time(time_ms)? - off;
    let end = time_ms.len().min(23);
    let frac = match (time_ms.get(20..end), end > 20) {
        (Some(d), true) if !d.is_empty() => d
            .parse::<i64>()
            .map_err(|_| McpError::Parse(format!("time_ms has no ms digits: {time_ms}")))?,
        _ => 0,
    };
    Ok(base * 1000 + frac)
}

/// The newest tick in the last 3 s as UTC ms, or `None` when the market is quiet.
pub async fn last_tick<M: Terminal>(
    misc: &M,
    sym: &str,
    off: i64,
) -> Result<Option<i64>, McpError> {
    let now = now_f64();
    let j = misc
        .call(
            "get_chart_ticks_history",
            json!({
                "symbol": sym,
                "datetime_from": iso((now - 3.0 + off as f64).floor() as i64),
                "datetime_to": iso((now + 60.0 + off as f64).floor() as i64),
            }),
        )
        .await?;
    let rows = j.get("history").and_then(Value::as_array);
    let Some(last) = rows.and_then(|r| r.last()) else {
        return Ok(None);
    };
    let time_ms = last
        .get("time_ms")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::Parse("the tick row has no time_ms".to_string()))?;
    tick_ms(time_ms, off).map(Some)
}

/// Every symbol, heartbeat rows dropped, category mapped to the hub's path.
pub async fn symbols<M: Terminal>(misc: &M) -> Result<Vec<Value>, McpError> {
    let j = misc
        .call(
            "get_marketwatch_symbols",
            json!({"include_hidden": true, "limit": 5000}),
        )
        .await?;
    let rows = j.get("symbols").and_then(Value::as_array);
    let mut out = Vec::with_capacity(rows.map_or(0, Vec::len));
    for s in rows.into_iter().flatten() {
        let category = s
            .get("category")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase();
        if category == "heartbeat" {
            continue;
        }
        let path = CATEGORY_PATH
            .iter()
            .find(|(k, _)| *k == category)
            .map_or("Other", |(_, v)| *v);
        out.push(json!({
            "name": s.get("symbol").cloned().unwrap_or(Value::Null),
            "desc": s.get("description").cloned().unwrap_or_else(|| json!("")),
            "digits": s.get("digits").cloned().unwrap_or_else(|| json!(2)),
            "path": path,
        }));
    }
    Ok(out)
}

/// The broker's server name, after refusing a terminal that answers but is not logged in: its
/// candles would be frozen, so the oracle raises this exact text.
pub async fn account_health<M: Terminal>(misc: &M) -> Result<Option<String>, McpError> {
    let j = misc.call("get_trading_account_info", json!({})).await?;
    let account = j
        .get("account")
        .ok_or_else(|| McpError::Parse("the account answer has no account".to_string()))?;
    let truthy = |v: Option<&Value>| match v {
        Some(Value::Null) | None => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    };
    if truthy(account.get("balance")) && !truthy(account.get("equity")) {
        return Err(McpError::Terminal(
            "MT5 terminal is not logged in".to_string(),
        ));
    }
    Ok(account
        .get("server")
        .and_then(Value::as_str)
        .map(str::to_string))
}

/// Python's `round()`: half-to-even. `d/1800` lands exactly on `.5` whenever a server clock is
/// 15 minutes off, and half-away-from-zero (Rust's `f64::round`) would pick the other slot.
fn round_half_even(x: f64) -> f64 {
    let floor = x.floor();
    let frac = x - floor;
    // Up above the tie, and at the tie only when the floor is odd (Python's round()).
    if frac > 0.5 || (frac == 0.5 && (floor / 2.0).fract() != 0.0) {
        floor + 1.0
    } else {
        floor
    }
}

/// The oracle's `measure_offset()`: server time minus UTC, rounded to 30-minute steps, refused
/// when the remainder is 90 s or more (the server clock is then stale, and the old offset stands).
pub async fn offset_measure<M: Terminal>(misc: &M) -> Result<Option<Measured>, McpError> {
    let t = misc.call("get_time_information", json!({})).await?;
    let field = |name: &str| -> Result<&str, McpError> {
        t.get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::Parse(format!("time information has no {name}")))
    };
    let d = parse_time(field("trade_server_last_known_time")?)? - parse_time(field("utc_time")?)?;
    let h = (round_half_even(d as f64 / 1800.0) as i64) * 1800;
    if (d - h).abs() >= 90 {
        return Ok(None);
    }
    Ok(Some(Measured {
        offset_s: h,
        measured_at: now_secs(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    use serde_json::json;

    /// An in-memory terminal: a bar grid aligned to `sec` in the half-open window the caller asks
    /// for, plus a fixed tick list. It records every call so the tests can read the windows.
    struct Grid {
        sec: i64,
        /// The newest bar the grid can answer.
        newest: i64,
        /// At most this many rows per answer (what a real terminal does on a short window).
        max_rows: usize,
        ticks: Vec<Value>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl Grid {
        fn new(sec: i64, newest: i64, max_rows: usize) -> Self {
            Self {
                sec,
                newest,
                max_rows,
                ticks: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Terminal for Grid {
        fn call(
            &self,
            name: &str,
            args: Value,
        ) -> impl Future<Output = Result<Value, McpError>> + Send {
            self.calls
                .lock()
                .unwrap()
                .push((name.to_string(), args.clone()));
            let sec = self.sec;
            let newest = self.newest;
            let max_rows = self.max_rows;
            let ticks = self.ticks.clone();
            async move {
                match name {
                    "get_chart_history" => {
                        let from = parse_time(args["datetime_from"].as_str().unwrap())?;
                        let to = parse_time(args["datetime_to"].as_str().unwrap())?;
                        let last = (to.min(newest) / sec) * sec;
                        let first = ((from / sec) * sec).max(last - (max_rows as i64 - 1) * sec);
                        let mut rows = Vec::new();
                        let mut t = first;
                        while t <= last {
                            rows.push(json!({
                                "time": iso(t),
                                "open": t as f64,
                                "high": t as f64 + 1.0,
                                "low": t as f64 - 1.0,
                                "close": t as f64 + 0.5,
                                "tick_volume": 10.0,
                            }));
                            t += sec;
                        }
                        Ok(json!({"history": rows}))
                    }
                    "get_chart_ticks_history" => Ok(json!({"history": ticks})),
                    other => Err(McpError::Terminal(format!("no tool {other}"))),
                }
            }
        }
    }

    const NEWEST: i64 = 1_764_150_000; // 2025-11-26T09:40:00Z, a multiple of 60

    fn fast() -> SyncPolicy {
        SyncPolicy {
            cap: Duration::from_millis(200),
            step: Duration::from_millis(10),
        }
    }

    #[tokio::test]
    async fn tf_parses_exactly_the_seven_and_rejects_the_rest() {
        for (name, sec) in [
            ("M1", 60),
            ("M5", 300),
            ("M15", 900),
            ("M30", 1800),
            ("H1", 3600),
            ("H4", 14400),
            ("D1", 86400),
        ] {
            let tf = Tf::try_from(name).expect("a known timeframe");
            assert_eq!(tf.sec(), sec, "{name}");
            assert_eq!(tf.as_str(), name);
        }
        assert_eq!(
            Tf::try_from("M2").expect_err("M2 is unknown"),
            "unknown timeframe M2"
        );
        assert_eq!(
            Tf::try_from("m1").expect_err("case matters"),
            "unknown timeframe m1"
        );
    }

    #[test]
    fn py_str_matches_python_str_of_float() {
        // Every expected string was measured with python -I -c "print(repr(str(v)))".
        let cases: [(f64, &str); 18] = [
            (1829.0, "1829.0"),
            (2650.35, "2650.35"),
            (2649.9, "2649.9"),
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (0.1 + 0.2, "0.30000000000000004"),
            (0.0001, "0.0001"),
            (9.999e-5, "9.999e-05"),
            (1e-5, "1e-05"),
            (-1e-5, "-1e-05"),
            (1e15, "1000000000000000.0"),
            (9999999999999998.0, "9999999999999998.0"),
            (1e16, "1e+16"),
            (1.0000000000000002e16, "1.0000000000000002e+16"),
            (1e21, "1e+21"),
            (1.5e300, "1.5e+300"),
            (5e-324, "5e-324"),
            (1.2345678901234567, "1.2345678901234567"),
        ];
        for (v, want) in cases {
            assert_eq!(py_str(v), want, "{v:?}");
        }
        assert_eq!(py_str(f64::INFINITY), "inf");
        assert_eq!(py_str(f64::NEG_INFINITY), "-inf");
        assert_eq!(py_str(f64::NAN), "nan");
    }

    #[test]
    fn bar_key_uses_the_oracle_field_order_and_formatting() {
        let bar = Bar {
            time: 1_764_150_000,
            open: 2650.0,
            high: 2649.9,
            low: 2648.35,
            close: 2650.35,
            volume: 1829.0,
        };
        assert_eq!(
            bar_key(Some(&bar)),
            "1764150000,2650.35,2649.9,2648.35,1829.0"
        );
        assert_eq!(bar_key(None), "");
    }

    #[test]
    fn iso_and_parse_time_round_trip() {
        assert_eq!(iso(NEWEST), "2025-11-26T09:40:00");
        assert_eq!(parse_time("2025-11-26T09:40:00").unwrap(), NEWEST);
        assert_eq!(parse_time("2025-11-26 09:40:00").unwrap(), NEWEST);
        assert_eq!(parse_time("2025-11-26T09:40:00.123").unwrap(), NEWEST);
        assert!(parse_time("2025-11-26").is_err());
        assert!(parse_time("2025-11-26X09:40:00").is_err());
    }

    #[test]
    fn tick_ms_slices_characters_20_to_23() {
        assert_eq!(
            tick_ms("2025-11-26T09:40:00.123", 0).unwrap(),
            NEWEST * 1000 + 123
        );
        assert_eq!(tick_ms("2025-11-26T09:40:00", 0).unwrap(), NEWEST * 1000);
        // The oracle takes the digits as they stand: ".12" is 12 ms, not 120.
        assert_eq!(
            tick_ms("2025-11-26T09:40:00.12", 0).unwrap(),
            NEWEST * 1000 + 12
        );
        assert_eq!(
            tick_ms("2025-11-26T09:40:00.999", 3600).unwrap(),
            (NEWEST - 3600) * 1000 + 999
        );
    }

    #[test]
    fn offset_rounds_half_to_even_and_refuses_a_stale_clock() {
        assert_eq!(round_half_even(0.0), 0.0);
        assert_eq!(round_half_even(0.5), 0.0, "python round(0.5) == 0");
        assert_eq!(round_half_even(1.5), 2.0, "python round(1.5) == 2");
        assert_eq!(round_half_even(2.5), 2.0, "python round(2.5) == 2");
        assert_eq!(round_half_even(-0.5), 0.0);
        assert_eq!(round_half_even(-1.5), -2.0);
        assert_eq!(round_half_even(2.4), 2.0);
        assert_eq!(round_half_even(2.6), 3.0);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let measure = |server: &'static str, utc: &'static str| {
            let fake = Grid::new(60, NEWEST, 1);
            let t = json!({"trade_server_last_known_time": server, "utc_time": utc});
            rt.block_on(async {
                let misc = Misc::new(t);
                let _ = fake;
                offset_measure(&misc).await
            })
        };
        // Exactly three hours: the Thai broker clock the owner's host reports.
        let m = measure("2025-11-26T12:40:00", "2025-11-26T09:40:00")
            .unwrap()
            .expect("a measured offset");
        assert_eq!(m.offset_s, 10800);
        // 15 minutes off: outside the 90 s window, so refused — the half-to-even tie is
        // unreachable through this gate and is pinned by the unit test above.
        assert_eq!(
            measure("2025-11-26T09:55:00", "2025-11-26T09:40:00").unwrap(),
            None
        );
        // 110 s off a slot: refused, the old offset stands.
        assert_eq!(
            measure("2025-11-26T12:41:50", "2025-11-26T09:40:00").unwrap(),
            None
        );
        // 80 s off a slot: accepted, exactly the Python rule (|d-h| < 90).
        let m = measure("2025-11-26T12:41:20", "2025-11-26T09:40:00")
            .unwrap()
            .expect("a measured offset");
        assert_eq!(m.offset_s, 10800);
    }

    /// A terminal that answers one canned `get_time_information` body.
    struct Misc {
        body: Value,
    }

    impl Misc {
        fn new(body: Value) -> Self {
            Self { body }
        }
    }

    impl Terminal for Misc {
        fn call(
            &self,
            _name: &str,
            _args: Value,
        ) -> impl Future<Output = Result<Value, McpError>> + Send {
            let body = self.body.clone();
            async move { Ok(body) }
        }
    }

    #[tokio::test]
    async fn bars_trims_to_limit_and_clamps_at_five_thousand() {
        let fake = Grid::new(60, NEWEST, 6000);
        let out = bars(
            &fake,
            "XAUUSDm",
            Tf::M1,
            99_999,
            Some(NEWEST * 1000),
            0,
            &fast(),
        )
        .await
        .expect("bars");
        assert_eq!(out.len(), 5000, "99999 clamps to 5000");
        assert_eq!(out.last().unwrap().time, NEWEST, "newest last");
        assert_eq!(out.first().unwrap().time, NEWEST - 4999 * 60);
        // The window is built from the clamped limit: 5000*60*1.5 + 4 days.
        let calls = fake.calls();
        let span = parse_time(calls[0].1["datetime_to"].as_str().unwrap()).unwrap()
            - parse_time(calls[0].1["datetime_from"].as_str().unwrap()).unwrap();
        assert_eq!(span, 5000 * 60 * 3 / 2 + 4 * 86400);
        assert_eq!(calls[0].1["period"], "M1");
        assert_eq!(calls.len(), 1, "a full answer returns at once");
    }

    #[tokio::test]
    async fn bars_triples_the_span_while_the_answer_is_short() {
        let fake = Grid::new(60, NEWEST, 50);
        let out = bars(
            &fake,
            "XAUUSDm",
            Tf::M1,
            100,
            Some(NEWEST * 1000),
            0,
            &fast(),
        )
        .await
        .expect("bars");
        let calls = fake.calls();
        let span = |v: &Value| {
            parse_time(v["datetime_to"].as_str().unwrap()).unwrap()
                - parse_time(v["datetime_from"].as_str().unwrap()).unwrap()
        };
        assert!(calls.len() >= 2, "a short answer reads again");
        assert_eq!(span(&calls[0].1), 100 * 60 * 3 / 2 + 4 * 86400);
        assert_eq!(span(&calls[1].1), (100 * 60 * 3 / 2 + 4 * 86400) * 3);
        assert!(out.len() <= 50, "the fake has 50 rows");
    }

    #[tokio::test]
    async fn bars_keeps_reading_while_the_candle_is_stale() {
        // The newest bar is an hour behind the newest tick: the oracle calls this stale and reads
        // again until the cap runs out. No `to`, so the tick is consulted.
        let mut fake = Grid::new(60, NEWEST - 3600, 500);
        fake.ticks = vec![json!({"time_ms": iso(NEWEST)})];
        let out = bars(&fake, "XAUUSDm", Tf::M1, 10, None, 0, &fast())
            .await
            .expect("bars");
        assert!(
            fake.calls().len() > 2,
            "a stale candle keeps the loop going"
        );
        assert_eq!(out.len(), 10, "the trim still applies");
        assert_eq!(out.last().unwrap().time, NEWEST - 3600);
    }

    #[tokio::test]
    async fn last_tick_reads_the_newest_row_and_handles_an_empty_history() {
        let mut fake = Grid::new(60, NEWEST, 1);
        fake.ticks = vec![json!({"time_ms": "2025-11-26T09:40:00.250"})];
        assert_eq!(
            last_tick(&fake, "EURUSD", 0).await.unwrap(),
            Some(NEWEST * 1000 + 250)
        );
        let calls = fake.calls();
        assert_eq!(calls[0].0, "get_chart_ticks_history");
        assert_eq!(calls[0].1["symbol"], "EURUSD");

        let quiet = Grid::new(60, NEWEST, 1);
        assert_eq!(last_tick(&quiet, "EURUSD", 0).await.unwrap(), None);
    }

    #[tokio::test]
    async fn symbols_maps_categories_and_drops_heartbeats() {
        let fake = Misc::new(json!({"symbols": [
            {"symbol": "EURUSD", "description": "Euro vs US Dollar", "digits": 5, "category": "Majors"},
            {"symbol": "BTCUSD", "description": "Bitcoin", "digits": 2, "category": "crypto"},
            {"symbol": "XAUUSDm", "description": "Gold", "digits": 3, "category": "GOLD"},
            {"symbol": "HB", "description": "heartbeat", "digits": 0, "category": "HeartBeat"},
        ]}));
        let out = symbols(&fake).await.expect("symbols");
        assert_eq!(out.len(), 3, "the heartbeat row is dropped");
        assert_eq!(out[0]["name"], "EURUSD");
        assert_eq!(out[0]["path"], "Forex");
        assert_eq!(out[0]["digits"], 5);
        assert_eq!(out[1]["path"], "Crypto");
        assert_eq!(out[2]["path"], "Other", "an unknown category falls back");
    }

    #[tokio::test]
    async fn health_refuses_a_terminal_that_is_not_logged_in() {
        let frozen =
            Misc::new(json!({"account": {"balance": 1000.0, "server": "Exness-MT5Real28"}}));
        let err = account_health(&frozen).await.expect_err("no equity");
        assert_eq!(err.to_string(), "MT5 terminal is not logged in");

        let ok = Misc::new(json!({"account": {
            "balance": 1000.0, "equity": 1000.0, "server": "Exness-MT5Real28"
        }}));
        assert_eq!(
            account_health(&ok).await.unwrap().as_deref(),
            Some("Exness-MT5Real28")
        );

        // A zero balance is falsy in the oracle, so it does not raise.
        let flat = Misc::new(json!({"account": {"balance": 0.0, "equity": 0.0, "server": "x"}}));
        assert_eq!(account_health(&flat).await.unwrap().as_deref(), Some("x"));
    }

    #[test]
    fn the_mcp_surface_is_exactly_the_five_oracle_tools() {
        assert_eq!(TOOLS.len(), 5);
    }
}
