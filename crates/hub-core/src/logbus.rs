//! The one backend log, ported from `hub/logbus.py`: a 12,000-line ring served by `/api/logs`, a
//! rotating file `hub.log` (2 MB x 3) reloaded at boot, redaction before anything is stored, edge-
//! triggered state lines, and the `problems` map the panel shows as banners.
//!
//! `LogRing` is pure (the caller passes `now`); `LogBus` adds the lock and the file; `RingLayer`
//! turns `tracing` events into lines (D-006: target `server`, or target `feed` plus field `src`).

use hub_types::logs::{LogRow, LogSnapshot, SourceInfo};
use regex::Regex;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub const RING_CAP: usize = 12_000;
pub const SNAPSHOT_LIMIT: usize = 3_000;
const MSG_CAP: usize = 2_000;
const THROTTLE_CAP: usize = 2_000;
const FILE_MAX: u64 = 2_000_000;
const FILE_BACKUPS: usize = 3;
const RELOAD_BYTES: u64 = 1_000_000;
const RELOAD_LINES: usize = 4_000;

fn patterns() -> &'static [(Regex, &'static str); 4] {
    static P: OnceLock<[(Regex, &'static str); 4]> = OnceLock::new();
    P.get_or_init(|| {
        [
            (Regex::new(r"(?i)\b(bearer)\s+\S+").unwrap(), "$1 [redacted]"),
            (
                Regex::new(r"(?i)\b(authorization|token|api[_-]?key|apikey|secret|passw(?:or)?d|pwd)\b(\s*[:=]\s*|\s+)\S+").unwrap(),
                "$1=[redacted]",
            ),
            (Regex::new(r"\b[0-9a-fA-F]{32,}\b").unwrap(), "[redacted]"),
            (Regex::new(r"\b[A-Za-z0-9+_-]{40,}={0,2}").unwrap(), "[redacted]"),
        ]
    })
}

/// The four Python patterns, in order. Secrets never reach a log line.
pub fn redact(text: &str) -> String {
    let mut s = text.to_string();
    for (rx, rep) in patterns() {
        s = rx.replace_all(&s, *rep).into_owned();
    }
    s
}

/// Python's `str[:2000]` (characters, not bytes).
fn cap(s: String) -> String {
    if s.chars().count() <= MSG_CAP {
        s
    } else {
        s.chars().take(MSG_CAP).collect()
    }
}

/// One line to add.
#[derive(Clone, Debug)]
pub struct Line {
    pub src: String,
    pub lvl: String,
    pub msg: String,
    /// Event time; `None` means now.
    pub t_ms: Option<i64>,
    /// Drop the same `src` + `msg` within this many seconds.
    pub every: Option<u64>,
    /// Write to the file.
    pub persist: bool,
    /// The line comes from the source (refreshes its `last`); false for lines about it.
    pub alive: bool,
}

impl Line {
    pub fn new(src: impl Into<String>, lvl: &str, msg: impl Into<String>) -> Line {
        Line {
            src: src.into(),
            lvl: lvl.to_string(),
            msg: msg.into(),
            t_ms: None,
            every: None,
            persist: true,
            alive: true,
        }
    }

    pub fn every(mut self, secs: u64) -> Line {
        self.every = Some(secs);
        self
    }

    pub fn about(mut self) -> Line {
        self.alive = false;
        self
    }
}

/// The ring, the sources, the problems. Pure: every method takes `now`.
pub struct LogRing {
    boot: i64,
    rows: VecDeque<LogRow>,
    seq: u64,
    throttle: HashMap<(String, String), i64>,
    sources: BTreeMap<String, SourceInfo>,
    problems: BTreeMap<String, String>,
    states: HashMap<(String, String), String>,
    /// 07-03 AC-5: secrets registered for literal redaction (bridge tokens, bridge URL query
    /// strings). Applied after the static set, in `push`.
    secrets: Vec<String>,
}

impl LogRing {
    pub fn new(boot_s: i64) -> LogRing {
        LogRing {
            boot: boot_s,
            rows: VecDeque::with_capacity(1024),
            seq: 0,
            throttle: HashMap::new(),
            sources: BTreeMap::new(),
            problems: BTreeMap::new(),
            states: HashMap::new(),
            secrets: Vec::new(),
        }
    }

    /// Registers a secret for literal redaction in `push` (07-03 AC-5). Four bytes minimum so
    /// a tiny fragment cannot eat the ring; duplicates are harmless.
    pub fn add_secret(&mut self, secret: &str) {
        if secret.len() >= 4 {
            self.secrets.push(secret.to_string());
        }
    }

    fn source(&mut self, src: &str) -> &mut SourceInfo {
        self.sources
            .entry(src.to_string())
            .or_insert_with(|| SourceInfo {
                last: 0,
                expect: None,
                state: "ok".into(),
                note: String::new(),
            })
    }

    /// Adds a line (redacted, capped, throttled). Returns the stored row, or `None` when throttled.
    pub fn push(&mut self, now_ms: i64, line: Line) -> Option<LogRow> {
        let msg = cap(redact(&line.msg));
        // 07-03 AC-5: the bridge tokens and URL query strings registered on this bus join the
        // static set, so a hostile bridge echoing the Authorization value into its error text
        // cannot land it in the ring (07-01's tracked note, closed here).
        let msg = if self.secrets.is_empty() {
            msg
        } else {
            let mut m = msg;
            for s in &self.secrets {
                if m.contains(s.as_str()) {
                    m = m.replace(s.as_str(), "[redacted]");
                }
            }
            m
        };
        let t = line.t_ms.unwrap_or(now_ms);
        if let Some(every) = line.every.filter(|e| *e > 0) {
            let k = (line.src.clone(), msg.clone());
            if now_ms - self.throttle.get(&k).copied().unwrap_or(i64::MIN / 2) < every as i64 * 1000
            {
                return None;
            }
            if self.throttle.len() > THROTTLE_CAP {
                self.throttle.clear();
            }
            self.throttle.insert(k, now_ms);
        }
        self.seq += 1;
        let row = LogRow {
            id: self.seq,
            t,
            src: line.src.clone(),
            lvl: line.lvl.clone(),
            msg,
        };
        self.rows.push_back(row.clone());
        while self.rows.len() > RING_CAP {
            self.rows.pop_front();
        }
        let alive = line.alive;
        let s = self.source(&line.src);
        if alive {
            s.last = s.last.max(t);
        }
        Some(row)
    }

    /// Logs only when `value` changes for `(src, key)` (one line per state change).
    pub fn state(
        &mut self,
        now_ms: i64,
        src: &str,
        key: &str,
        value: &str,
        lvl: &str,
        msg: &str,
    ) -> Option<LogRow> {
        let k = (src.to_string(), key.to_string());
        if self.states.get(&k).map(String::as_str) == Some(value) {
            return None;
        }
        self.states.insert(k, value.to_string());
        self.push(now_ms, Line::new(src, lvl, msg))
    }

    /// Replaces every problem whose key starts with `prefix` by `desired` (derive, never sync).
    pub fn replace_problems(&mut self, prefix: &str, desired: BTreeMap<String, String>) {
        self.problems.retain(|k, _| !k.starts_with(prefix));
        self.problems.extend(desired);
    }

    pub fn snapshot(&self, now_ms: i64, since: u64, brief: bool) -> LogSnapshot {
        let recent: Vec<&LogRow> = self
            .rows
            .iter()
            .filter(|r| r.lvl == "error" && now_ms - r.t < 900_000)
            .collect();
        let lines = (!brief).then(|| {
            let newer: Vec<LogRow> = self.rows.iter().filter(|r| r.id > since).cloned().collect();
            let skip = newer.len().saturating_sub(SNAPSHOT_LIMIT);
            newer.into_iter().skip(skip).collect()
        });
        LogSnapshot {
            boot: self.boot,
            last: self.seq,
            now: now_ms,
            errors_15m: recent.len(),
            latest_error: recent.last().map(|r| (*r).clone()),
            problems: self.problems.clone(),
            sources: self.sources.clone(),
            lines,
        }
    }

    /// Re-adds lines read back from the file at boot, with fresh ids.
    pub fn reload(&mut self, rows: Vec<(i64, String, String, String)>) {
        for (t, lvl, src, msg) in rows {
            self.seq += 1;
            self.rows.push_back(LogRow {
                id: self.seq,
                t,
                src: src.clone(),
                lvl,
                msg,
            });
            let s = self.source(&src);
            s.last = s.last.max(t);
        }
        while self.rows.len() > RING_CAP {
            self.rows.pop_front();
        }
    }
}

/// `2026-11-23T09:27:30.635Z` from epoch ms. Pure.
pub fn iso(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // civil from days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{milli:03}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// Inverse of `iso` for reload. Pure.
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z').unwrap_or(s);
    if s.len() != 23 {
        return None;
    }
    let num = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se, ms) = (
        num(0, 4)?,
        num(5, 7)?,
        num(8, 10)?,
        num(11, 13)?,
        num(14, 16)?,
        num(17, 19)?,
        num(20, 23)?,
    );
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = if mo > 2 { mo - 3 } else { mo + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + se) * 1000) + ms)
}

/// The file line Python writes: `"%s %-5s %s %s" % (iso(t), level, src, msg_without_newlines)`.
pub fn file_line(row: &LogRow) -> String {
    format!(
        "{} {:<5} {} {}",
        iso(row.t),
        row.lvl,
        row.src,
        row.msg.replace('\n', " ")
    )
}

/// Parses one file line back into `(t, lvl, src, msg)`.
pub fn parse_file_line(line: &str) -> Option<(i64, String, String, String)> {
    static RX: OnceLock<Regex> = OnceLock::new();
    let rx = RX.get_or_init(|| {
        Regex::new(r"^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3})Z (\w+)\s+(\S+) (.*)$").unwrap()
    });
    let c = rx.captures(line)?;
    let iso_text = format!("{}Z", &c[1]);
    Some((
        parse_iso(&iso_text[..23])?,
        c[2].to_string(),
        c[3].to_string(),
        c[4].to_string(),
    ))
}

/// Appends to `dir/hub.log`, rotating at 2 MB into `hub.log.1` to `hub.log.3`.
pub struct FileSink {
    path: PathBuf,
    /// Serialises rotate plus append, so concurrent lines never interleave.
    lock: Mutex<()>,
}

impl FileSink {
    pub fn new(dir: &Path) -> std::io::Result<FileSink> {
        fs::create_dir_all(dir)?;
        Ok(FileSink {
            path: dir.join("hub.log"),
            lock: Mutex::new(()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, line: &str) -> std::io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0) + line.len() as u64 + 1 > FILE_MAX
        {
            self.rotate()?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(
            format!(
                "{line}
"
            )
            .as_bytes(),
        )
    }

    fn rotate(&self) -> std::io::Result<()> {
        let name = |i: usize| self.path.with_file_name(format!("hub.log.{i}"));
        let _ = fs::remove_file(name(FILE_BACKUPS));
        for i in (1..FILE_BACKUPS).rev() {
            let _ = fs::rename(name(i), name(i + 1));
        }
        fs::rename(&self.path, name(1))
    }

    /// The last ~1 MB of the file, first (partial) line dropped as Python does, last 4,000 lines.
    pub fn tail(&self) -> Vec<(i64, String, String, String)> {
        let Ok(mut f) = fs::File::open(&self.path) else {
            return Vec::new();
        };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        let _ = f.seek(SeekFrom::Start(len.saturating_sub(RELOAD_BYTES)));
        let mut buf = Vec::new();
        let _ = f.read_to_end(&mut buf);
        let text = String::from_utf8_lossy(&buf);
        let lines: Vec<&str> = text.lines().skip(1).collect();
        let start = lines.len().saturating_sub(RELOAD_LINES);
        lines[start..]
            .iter()
            .filter_map(|l| parse_file_line(l))
            .collect()
    }
}

/// The ring behind a lock, plus the file. Cheap to clone.
#[derive(Clone)]
pub struct LogBus {
    inner: Arc<BusInner>,
}

struct BusInner {
    ring: Mutex<LogRing>,
    file: Option<FileSink>,
    clock: crate::clock::Clock,
}

impl LogBus {
    /// A bus with an optional file in `dir` (reloaded first). `boot_s` is the process start.
    pub fn new(dir: Option<&Path>, boot_s: i64, clock: crate::clock::Clock) -> LogBus {
        let mut ring = LogRing::new(boot_s);
        let file = dir.and_then(|d| FileSink::new(d).ok());
        if let Some(f) = &file {
            ring.reload(f.tail());
        }
        LogBus {
            inner: Arc::new(BusInner {
                ring: Mutex::new(ring),
                file,
                clock,
            }),
        }
    }

    fn now(&self) -> i64 {
        self.inner.clock.true_now_ms() as i64
    }

    fn ring(&self) -> std::sync::MutexGuard<'_, LogRing> {
        self.inner.ring.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn push(&self, line: Line) {
        let persist = line.persist;
        let now = self.now();
        let row = self.ring().push(now, line);
        if let (Some(row), Some(f), true) = (row, &self.inner.file, persist) {
            let _ = f.write(&file_line(&row));
        }
    }

    /// Registers a secret that must never appear in the ring (07-03 AC-5): bridge tokens and
    /// bridge URL query strings join the redaction set. Shared by every clone of the bus.
    pub fn add_secret(&self, secret: &str) {
        if secret.len() >= 4 {
            self.inner
                .ring
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .add_secret(secret);
        }
    }

    pub fn state(&self, src: &str, key: &str, value: &str, lvl: &str, msg: &str) {
        let now = self.now();
        let row = self.ring().state(now, src, key, value, lvl, msg);
        if let (Some(row), Some(f)) = (row, &self.inner.file) {
            let _ = f.write(&file_line(&row));
        }
    }

    pub fn replace_problems(&self, prefix: &str, desired: BTreeMap<String, String>) {
        self.ring().replace_problems(prefix, desired);
    }

    pub fn snapshot(&self, since: u64, brief: bool) -> LogSnapshot {
        let now = self.now();
        self.ring().snapshot(now, since, brief)
    }
}

/// `tracing` events into the ring (D-006). Target `feed` plus field `src` gives `feed.<src>`;
/// target `server` (and any other target) gives `server`. Fields `every`, `persist` and `alive`
/// steer the line; other fields are appended as ` k=v`.
pub struct RingLayer {
    bus: LogBus,
}

impl RingLayer {
    pub fn new(bus: LogBus) -> RingLayer {
        RingLayer { bus }
    }
}

#[derive(Default)]
struct Fields {
    message: String,
    src: Option<String>,
    every: Option<u64>,
    persist: Option<bool>,
    alive: Option<bool>,
    facts: Vec<String>,
}

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = format!("{value:?}"),
            "src" => self.src = Some(format!("{value:?}")),
            "every" => self.every = format!("{value:?}").parse().ok(),
            name => self.facts.push(format!("{name}={value:?}")),
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.message = value.to_string(),
            "src" => self.src = Some(value.to_string()),
            name => self.facts.push(format!("{name}={value}")),
        }
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        match field.name() {
            "every" => self.every = Some(value),
            name => self.facts.push(format!("{name}={value}")),
        }
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        match field.name() {
            "every" => self.every = u64::try_from(value).ok(),
            name => self.facts.push(format!("{name}={value}")),
        }
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        match field.name() {
            "persist" => self.persist = Some(value),
            "alive" => self.alive = Some(value),
            name => self.facts.push(format!("{name}={value}")),
        }
    }
}

/// Log-panel source for a `tracing` target and `src` field (D-006). Pure.
pub fn source_of(target: &str, src: Option<&str>) -> String {
    match (target, src) {
        ("feed", Some(s)) => format!("feed.{s}"),
        ("feed", None) => "feed".to_string(),
        _ => "server".to_string(),
    }
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RingLayer {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        if meta.target() != "feed"
            && meta.target() != "server"
            && *meta.level() > tracing::Level::WARN
        {
            return; // library chatter below warn stays out of the panel
        }
        let mut f = Fields::default();
        event.record(&mut f);
        let lvl = match *meta.level() {
            tracing::Level::ERROR => "error",
            tracing::Level::WARN => "warn",
            tracing::Level::INFO => "info",
            _ => "debug",
        };
        let mut msg = f.message;
        if !f.facts.is_empty() {
            msg = format!("{msg} {}", f.facts.join(" "));
        }
        let mut line = Line::new(source_of(meta.target(), f.src.as_deref()), lvl, msg);
        line.every = f.every;
        line.persist = f.persist.unwrap_or(true);
        line.alive = f.alive.unwrap_or(true);
        self.bus.push(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_secrets_never_reach_the_ring() {
        // AC-5: the token and the URL's query string, registered on the ring, vanish from any
        // line a hostile bridge could have echoed them into.
        let mut ring = LogRing::new(0);
        ring.add_secret("tok-bridge-42");
        ring.add_secret("token=abc&x=1");
        let row = ring
            .push(
                0,
                Line::new(
                    "server",
                    "info",
                    "bridge said: Bearer tok-bridge-42 at /bars?token=abc&x=1 end",
                ),
            )
            .expect("stored");
        assert!(!row.msg.contains("tok-bridge-42"), "{}", row.msg);
        assert!(!row.msg.contains("token=abc&x=1"), "{}", row.msg);
        assert!(row.msg.contains("[redacted]"), "{}", row.msg);
    }

    #[test]
    fn redaction_matches_python() {
        // expected strings produced by hub/logbus.py redact() for the same inputs
        let cases = [
            (
                "Authorization: Bearer abc.def",
                "Authorization=[redacted] [redacted]",
            ),
            ("token: x1 done", "token=[redacted] done"),
            (
                "id 0123456789abcdef0123456789abcdef end",
                "id [redacted] end",
            ),
            (
                "jwt eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9xyzxyzxyz ok",
                "jwt [redacted] ok",
            ),
            ("the token expired", "the token=[redacted]"),
            ("Bybit: 3403 charts loaded", "Bybit: 3403 charts loaded"),
        ];
        for (input, want) in cases {
            assert_eq!(redact(input), want, "{input}");
        }
    }

    #[test]
    fn throttle_cap_and_ring_limits() {
        let mut r = LogRing::new(1);
        assert!(r
            .push(1_000, Line::new("feed.bybit", "warn", "x").every(30))
            .is_some());
        assert!(
            r.push(20_000, Line::new("feed.bybit", "warn", "x").every(30))
                .is_none(),
            "same line within 30 s"
        );
        assert!(r
            .push(31_001, Line::new("feed.bybit", "warn", "x").every(30))
            .is_some());
        let long = "a ".repeat(1_500);
        assert_eq!(
            r.push(40_000, Line::new("server", "info", long))
                .unwrap()
                .msg
                .chars()
                .count(),
            2_000
        );
        for i in 0..RING_CAP + 10 {
            r.push(
                50_000 + i as i64,
                Line::new("server", "info", format!("n{i}")),
            );
        }
        let s = r.snapshot(60_000, 0, false);
        assert_eq!(s.lines.as_ref().unwrap().len(), SNAPSHOT_LIMIT);
        assert!(r.snapshot(60_000, 0, true).lines.is_none());
    }

    #[test]
    fn errors_states_and_problems() {
        let mut r = LogRing::new(7);
        r.push(1_000, Line::new("server", "error", "boom"));
        assert!(r
            .state(2_000, "feed.deriv", "ok", "true", "info", "up: 43 symbols")
            .is_some());
        assert!(r
            .state(3_000, "feed.deriv", "ok", "true", "info", "up: 43 symbols")
            .is_none());
        r.replace_problems(
            "feed.",
            BTreeMap::from([(
                "feed.binance:BTCUSDT M1".to_string(),
                "no data for over 120 s".to_string(),
            )]),
        );
        let s = r.snapshot(10_000, 0, true);
        assert_eq!((s.boot, s.errors_15m), (7, 1));
        assert_eq!(s.latest_error.unwrap().msg, "boom");
        assert_eq!(s.problems.len(), 1);
        r.replace_problems("feed.", BTreeMap::new());
        assert!(r.snapshot(10_000, 0, true).problems.is_empty());
        assert_eq!(
            r.snapshot(2_000_000, 0, true).errors_15m,
            0,
            "errors older than 15 min drop out"
        );
    }

    #[test]
    fn file_lines_round_trip() {
        assert_eq!(iso(1_795_440_813_058), "2026-11-23T13:33:33.058Z");
        assert_eq!(
            parse_iso("2026-11-23T13:33:33.058Z"),
            Some(1_795_440_813_058)
        );
        let row = LogRow {
            id: 1,
            t: 1_795_440_813_058,
            src: "server".into(),
            lvl: "info".into(),
            msg: "a\nb".into(),
        };
        let line = file_line(&row);
        assert_eq!(line, "2026-11-23T13:33:33.058Z info  server a b");
        assert_eq!(
            parse_file_line(&line),
            Some((
                1_795_440_813_058,
                "info".into(),
                "server".into(),
                "a b".into()
            ))
        );
        assert_eq!(source_of("feed", Some("bybit")), "feed.bybit");
        assert_eq!(source_of("hub_core::catalog", None), "server");
    }
}
