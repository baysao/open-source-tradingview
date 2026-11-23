//! Every Binance futures trade the charts asked for (`hub/orderflow_trades.py`, `hub/orderflow.py`).
//!
//! Past UTC days come from the daily archive (downloaded in the background, cached as D-008 column
//! files); recent hours come from REST `aggTrades`, newest first, with complete UTC hours saved so a
//! restart does not start over. Coverage is kept as exact time ranges.
//!
//! The pure parts (`Segments::add`, `assemble`, the head and back-fill plans, the codec, the CSV
//! parser) carry Python's rules and are tested against Python's own store. `SymbolStore` is the
//! shell: one short std lock around the state, never held across an await; its loops hold a `Weak`
//! so dropping the registry stops them.

use super::binance::{Fapi, Prio};
use super::calc::{py_max, py_min, union, TradeCols, TradeWindow};
use super::{OfError, PyClass};
use crate::clock::WallClock;
use futures_util::future::BoxFuture;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

pub const MAX_PAST_DAYS: i64 = 2;
pub const MAX_REST_HOURS: i64 = 30;
pub const POLL_S: f64 = 4.0;
pub const IDLE_STOP_S: f64 = 600.0;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 86_400_000;
const DAY_RETRY_S: f64 = 1800.0;

// ---------------------------------------------------------------- data

/// Trades with consecutive aggTrade ids, immutable once built.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chunk {
    pub ids: Vec<i64>,
    pub cols: TradeCols,
}

impl Chunk {
    /// The part of the chunk whose ids are above `last`.
    fn above(&self, last: i64) -> Option<Chunk> {
        let keep: Vec<usize> = (0..self.ids.len())
            .filter(|&i| self.ids[i] > last)
            .collect();
        if keep.is_empty() {
            return None;
        }
        let c = &self.cols;
        Some(Chunk {
            ids: keep.iter().map(|&i| self.ids[i]).collect(),
            cols: TradeCols {
                t_ms: keep.iter().map(|&i| c.t_ms[i]).collect(),
                price: keep.iter().map(|&i| c.price[i]).collect(),
                qty: keep.iter().map(|&i| c.qty[i]).collect(),
                side: keep.iter().map(|&i| c.side[i]).collect(),
            },
        })
    }
}

/// Chunks in id order plus the exact time range they fully cover.
#[derive(Clone, Debug, PartialEq)]
pub struct Seg {
    pub chunks: Vec<Arc<Chunk>>,
    pub t0: i64,
    pub t1: i64,
}

impl Seg {
    pub fn first(&self) -> i64 {
        self.chunks[0].ids[0]
    }

    pub fn last(&self) -> i64 {
        *self
            .chunks
            .last()
            .expect("a segment has a chunk")
            .ids
            .last()
            .expect("a chunk has an id")
    }
}

/// Segments sorted by first id, ids disjoint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Segments {
    pub segs: Vec<Seg>,
}

impl Segments {
    /// `_add`: insert trades with consecutive ids covering `[t0, t1]`, merging with neighbours whose
    /// ids touch. A join: adding the same chunks in any order, or twice, gives the same segments.
    pub fn add(&mut self, chunk: Chunk, t0: i64, t1: i64) {
        if chunk.ids.is_empty() {
            return;
        }
        self.segs.push(Seg {
            chunks: vec![Arc::new(chunk)],
            t0,
            t1,
        });
        self.segs.sort_by_key(Seg::first);
        let mut out: Vec<Seg> = Vec::with_capacity(self.segs.len());
        for s in self.segs.drain(..) {
            match out.last_mut() {
                Some(cur) if s.first() <= cur.last() + 1 => {
                    for c in &s.chunks {
                        if let Some(rest) = c.above(cur.last()) {
                            cur.chunks.push(Arc::new(rest));
                        }
                    }
                    cur.t0 = cur.t0.min(s.t0);
                    cur.t1 = cur.t1.max(s.t1);
                }
                _ => out.push(s),
            }
        }
        self.segs = out;
    }
}

/// A day's archive as columns.
pub type DayCols = Arc<TradeCols>;

/// What `assemble` reads: the store's state at one moment.
#[derive(Clone, Debug, Default)]
pub struct StoreState {
    pub segs: Segments,
    pub floor_ms: Option<f64>,
    pub days: BTreeMap<i64, DayCols>,
    pub day_busy: BTreeSet<i64>,
    /// Wall seconds of the last failure per day.
    pub day_fail: HashMap<i64, f64>,
}

/// What a `trades()` call asks the shell to do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Effects {
    pub floor_ms: Option<f64>,
    /// Days to download (marked busy).
    pub fetch_days: Vec<i64>,
    /// The REST loops must run.
    pub start_loops: bool,
}

fn utc_day(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS)
}

/// `TradeStore.trades(t0, t1)` without side effects: trades fully loaded inside `[t0, t1]` (day
/// files by date, then segments, a trade inside a day file's range never twice), the ranges they
/// cover (unioned), whether more is coming, and the effects for the shell.
pub fn assemble(st: &StoreState, t0_ms: i64, t1_ms: i64, now_ms: f64) -> (TradeWindow, Effects) {
    let today = utc_day(now_ms.floor() as i64);
    let mut out = TradeCols::default();
    let mut cov: Vec<(i64, i64)> = Vec::new();
    let mut pending = false;
    let mut fx = Effects::default();
    let push = |out: &mut TradeCols, c: &TradeCols, keep: &dyn Fn(i64) -> bool| {
        for i in 0..c.len() {
            if keep(c.t_ms[i]) {
                out.t_ms.push(c.t_ms[i]);
                out.price.push(c.price[i]);
                out.qty.push(c.qty[i]);
                out.side.push(c.side[i]);
            }
        }
    };
    let first = utc_day(t0_ms).max(today - MAX_PAST_DAYS);
    let last = utc_day(t1_ms).min(today - 1);
    for d in first..=last {
        if let Some(a) = st.days.get(&d) {
            push(&mut out, a, &|t| t >= t0_ms && t <= t1_ms);
            cov.push((d * DAY_MS, d * DAY_MS + DAY_MS - 1));
        } else if st.day_busy.contains(&d) {
            pending = true;
        } else if now_ms / 1000.0 - st.day_fail.get(&d).copied().unwrap_or(0.0) >= DAY_RETRY_S {
            fx.fetch_days.push(d);
            pending = true;
        }
    }
    let day_cov = cov.clone();
    let rest_from = py_max(t0_ms as f64, now_ms - (MAX_REST_HOURS * HOUR_MS) as f64);
    if t1_ms as f64 >= rest_from {
        fx.floor_ms = Some(st.floor_ms.map_or(rest_from, |f| py_min(f, rest_from)));
        fx.start_loops = true;
        for s in st
            .segs
            .segs
            .iter()
            .filter(|s| s.t1 >= t0_ms && s.t0 <= t1_ms)
        {
            let parts: Vec<&Arc<Chunk>> = s
                .chunks
                .iter()
                .filter(|c| {
                    c.cols.t_ms.last().is_some_and(|t| *t >= t0_ms) && c.cols.t_ms[0] <= t1_ms
                })
                .collect();
            for c in parts {
                push(&mut out, &c.cols, &|t| {
                    t >= t0_ms && t <= t1_ms && day_cov.iter().all(|&(lo, hi)| t < lo || t > hi)
                });
            }
            cov.push((s.t0, s.t1));
        }
        let end = py_min(t1_ms as f64, now_ms - 2000.0);
        if !covers_f(&cov, rest_from, end) {
            pending = true;
        }
    }
    (
        TradeWindow {
            trades: out,
            covered: union(&cov),
            pending,
        },
        fx,
    )
}

/// `_covers` with Python's float bounds (`rest_from` and `now - 2000` are floats there).
fn covers_f(iv: &[(i64, i64)], a: f64, b: f64) -> bool {
    b < a
        || union(iv)
            .iter()
            .any(|&(x, y)| x as f64 <= a && y as f64 >= b)
}

/// What the head poll asks for (`_poll_head`): from the newest segment's next id when it is fresh
/// (its coverage ends less than 2 minutes before `t_req`), else the newest page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeadPlan {
    pub from_id: Option<i64>,
    /// The newest segment's last id when fresh (identifies it after the await).
    pub fresh_last: Option<i64>,
}

pub fn head_plan(segs: &Segments, t_req: f64) -> HeadPlan {
    match segs.segs.last() {
        Some(h) if t_req - (h.t1 as f64) < 120_000.0 => HeadPlan {
            from_id: Some(h.last() + 1),
            fresh_last: Some(h.last()),
        },
        _ => HeadPlan {
            from_id: None,
            fresh_last: None,
        },
    }
}

/// Applies a head page. Returns whether another page follows at once (a fresh full page).
pub fn apply_head(segs: &mut Segments, plan: HeadPlan, page: Option<Chunk>, t_req: f64) -> bool {
    let fresh_seg = |segs: &mut Segments| -> Option<usize> {
        let last = plan.fresh_last?;
        segs.segs
            .iter()
            .position(|s| s.chunks.iter().any(|c| c.ids.contains(&last)))
    };
    let Some(c) = page else {
        if let Some(i) = fresh_seg(segs) {
            let s = &mut segs.segs[i];
            s.t1 = s.t1.max(t_req as i64);
        }
        return false;
    };
    let fresh = plan.fresh_last.is_some();
    let more = fresh && c.ids.len() == 1000;
    let first_t = c.cols.t_ms[0];
    let last_t = *c.cols.t_ms.last().expect("a page has trades");
    let t0 = if fresh {
        fresh_seg(segs).map_or(first_t + 1, |i| segs.segs[i].t1)
    } else {
        first_t + 1
    };
    let t1 = if more {
        last_t
    } else {
        py_max(t_req, last_t as f64) as i64
    };
    segs.add(c, t0, t1);
    more
}

/// What the back-fill asks for (`_back_step`): the newest missing stretch above `floor`, between
/// segments, else below the oldest; `None` when nothing is missing.
pub fn back_plan(segs: &Segments, floor: Option<f64>) -> Option<i64> {
    let floor = floor?;
    let s = &segs.segs;
    if s.is_empty() {
        return None;
    }
    for i in (1..s.len()).rev() {
        let (lo, hi) = (&s[i - 1], &s[i]);
        if hi.t0 as f64 > floor {
            return Some((lo.last() + 1).max(hi.first() - 1000));
        }
    }
    if s[0].t0 as f64 <= floor || s[0].first() == 0 {
        return None;
    }
    Some((s[0].first() - 1000).max(0))
}

// ---------------------------------------------------------------- D-008 codec and the daily archive

const MAGIC: &[u8; 4] = b"OFTR";
const VERSION: u16 = 1;

/// Day files carry no ids; hour files do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Day = 0,
    Hour = 1,
}

/// Header (magic, version, kind, rows) then little-endian columns; written to `.part` and renamed,
/// so a crash leaves either the old file or none.
pub fn write_bin(
    path: &Path,
    kind: FileKind,
    ids: Option<&[i64]>,
    t: &TradeCols,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut buf = Vec::with_capacity(16 + t.len() * 33);
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.push(kind as u8);
    buf.extend_from_slice(&(t.len() as u64).to_le_bytes());
    t.t_ms
        .iter()
        .for_each(|v| buf.extend_from_slice(&v.to_le_bytes()));
    t.price
        .iter()
        .for_each(|v| buf.extend_from_slice(&v.to_le_bytes()));
    t.qty
        .iter()
        .for_each(|v| buf.extend_from_slice(&v.to_le_bytes()));
    t.side.iter().for_each(|v| buf.push(*v as u8));
    if kind == FileKind::Hour {
        ids.unwrap_or(&[])
            .iter()
            .for_each(|v| buf.extend_from_slice(&v.to_le_bytes()));
    }
    let part = path.with_extension("part");
    {
        let mut f = std::fs::File::create(&part)?;
        f.write_all(&buf)?;
        f.sync_all()?;
    }
    std::fs::rename(&part, path)
}

/// Reads a file written by `write_bin`; any mismatch is an error (the caller skips the file).
pub fn read_bin(path: &Path) -> std::io::Result<(FileKind, Option<Vec<i64>>, TradeCols)> {
    let bad = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not an order-flow cache file",
        )
    };
    let b = std::fs::read(path)?;
    if b.len() < 15 || &b[..4] != MAGIC || u16::from_le_bytes([b[4], b[5]]) != VERSION {
        return Err(bad());
    }
    let kind = match b[6] {
        0 => FileKind::Day,
        1 => FileKind::Hour,
        _ => return Err(bad()),
    };
    let n = u64::from_le_bytes(b[7..15].try_into().map_err(|_| bad())?) as usize;
    let need = 15 + n * (8 + 8 + 8 + 1) + if kind == FileKind::Hour { n * 8 } else { 0 };
    if b.len() != need {
        return Err(bad());
    }
    let mut at = 15;
    let col8 = |at: &mut usize| -> Vec<[u8; 8]> {
        let v = (0..n)
            .map(|i| b[*at + i * 8..*at + i * 8 + 8].try_into().expect("8 bytes"))
            .collect();
        *at += n * 8;
        v
    };
    let t_ms = col8(&mut at).into_iter().map(i64::from_le_bytes).collect();
    let price = col8(&mut at).into_iter().map(f64::from_le_bytes).collect();
    let qty = col8(&mut at).into_iter().map(f64::from_le_bytes).collect();
    let side = b[at..at + n].iter().map(|v| *v as i8).collect();
    at += n;
    let ids = (kind == FileKind::Hour)
        .then(|| col8(&mut at).into_iter().map(i64::from_le_bytes).collect());
    Ok((
        kind,
        ids,
        TradeCols {
            t_ms,
            price,
            qty,
            side,
        },
    ))
}

/// The daily archive: the first entry's CSV, columns by name, side -1 when `is_buyer_maker` reads
/// `true` in any case (`fetch_day`).
pub fn parse_daily_zip(bytes: &[u8]) -> Result<TradeCols, OfError> {
    let bad = |m: &str| OfError::value(format!("daily archive: {m}"));
    let mut z =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| bad(&e.to_string()))?;
    let mut text = String::new();
    z.by_index(0)
        .map_err(|e| bad(&e.to_string()))?
        .read_to_string(&mut text)
        .map_err(|e| bad(&e.to_string()))?;
    let mut lines = text.lines();
    let head: Vec<&str> = lines.next().unwrap_or("").split(',').collect();
    let col = |name: &str| {
        head.iter()
            .position(|h| *h == name)
            .ok_or_else(|| OfError::new(PyClass::KeyError, format!("'{name}'")))
    };
    let (ct, cp, cq, cm) = (
        col("transact_time")?,
        col("price")?,
        col("quantity")?,
        col("is_buyer_maker")?,
    );
    let mut t = TradeCols::default();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let c: Vec<&str> = line.split(',').collect();
        t.t_ms.push(c[ct].trim().parse().map_err(|_| bad("time"))?);
        t.price
            .push(c[cp].trim().parse().map_err(|_| bad("price"))?);
        t.qty
            .push(c[cq].trim().parse().map_err(|_| bad("quantity"))?);
        t.side.push(if c[cm].trim().eq_ignore_ascii_case("true") {
            -1
        } else {
            1
        });
    }
    Ok(t)
}

/// `YYYY-MM-DD` of a day number (days since the epoch). Civil-from-days, no calendar crate.
pub fn day_str(d: i64) -> String {
    let (y, m, dd) = civil(d);
    format!("{y:04}-{m:02}-{dd:02}")
}

/// `YYYYMMDDHH` of an hour in ms.
pub fn hour_str(h_ms: i64) -> String {
    let (y, m, d) = civil(h_ms.div_euclid(DAY_MS));
    format!("{y:04}{m:02}{d:02}{:02}", h_ms.rem_euclid(DAY_MS) / HOUR_MS)
}

/// The hour in ms of a `YYYYMMDDHH` name.
pub fn parse_hour(s: &str) -> Option<i64> {
    if s.len() != 10 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |a: usize, b: usize| s[a..b].parse::<i64>().ok();
    let (y, m, d, h) = (n(0, 4)?, n(4, 6)?, n(6, 8)?, n(8, 10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || h > 23 {
        return None;
    }
    Some(days_from_civil(y, m, d) * DAY_MS + h * HOUR_MS)
}

fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ---------------------------------------------------------------- the shell

/// Where trades come from. `Fapi` is the real one; tests use an in-memory source, because tokio's
/// paused clock skips ahead while a task waits on a socket.
pub trait TradeSource: Send + Sync + 'static {
    fn agg_trades<'a>(
        &'a self,
        sym: &'a str,
        from_id: Option<i64>,
        p: Prio,
    ) -> BoxFuture<'a, Result<Option<Chunk>, OfError>>;
    fn daily_zip<'a>(
        &'a self,
        sym: &'a str,
        day: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, OfError>>;
}

impl TradeSource for Fapi {
    fn agg_trades<'a>(
        &'a self,
        sym: &'a str,
        from_id: Option<i64>,
        p: Prio,
    ) -> BoxFuture<'a, Result<Option<Chunk>, OfError>> {
        Box::pin(async move {
            Ok(Fapi::agg_trades(self, sym, from_id, p)
                .await?
                .map(|(ids, cols)| Chunk { ids, cols }))
        })
    }

    fn daily_zip<'a>(
        &'a self,
        sym: &'a str,
        day: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, OfError>> {
        Box::pin(Fapi::daily_zip(self, sym, day))
    }
}

struct Shared {
    st: StoreState,
    last_used: f64,
    running: bool,
    saved: HashSet<i64>,
}

/// All trades of one futures symbol that the charts asked for.
pub struct SymbolStore {
    sym: String,
    src: Arc<dyn TradeSource>,
    clock: WallClock,
    cache: Option<PathBuf>,
    shared: Mutex<Shared>,
    loaded: tokio::sync::OnceCell<()>,
}

impl SymbolStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn hour_dir(&self) -> Option<PathBuf> {
        self.cache.as_ref().map(|c| c.join("hours").join(&self.sym))
    }

    fn day_path(&self, d: i64) -> Option<PathBuf> {
        self.cache.as_ref().map(|c| {
            c.join("daily")
                .join(format!("{}-aggTrades-{}.bin", self.sym, day_str(d)))
        })
    }

    /// `_load_hours`: complete hours from disk; files older than the REST reach plus 3 h are removed.
    fn load_hours(&self) {
        let Some(dir) = self.hour_dir() else { return };
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        let oldest = self.clock.now_ms() - (MAX_REST_HOURS * HOUR_MS) as f64;
        let mut names: Vec<String> = rd
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.ends_with(".bin"))
            .collect();
        names.sort();
        let mut sh = self.lock();
        for n in names {
            let Some(h) = parse_hour(&n[..n.len() - 4]) else {
                continue;
            };
            let path = dir.join(&n);
            if (h as f64) < oldest - (3 * HOUR_MS) as f64 {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            if let Ok((FileKind::Hour, Some(ids), cols)) = read_bin(&path) {
                if !ids.is_empty() {
                    sh.st.segs.add(Chunk { ids, cols }, h, h + HOUR_MS - 1);
                    sh.saved.insert(h);
                }
            }
        }
    }

    /// `_save_hours`: every complete UTC hour inside a segment that is not on disk yet.
    fn save_hours(&self) {
        let Some(dir) = self.hour_dir() else { return };
        let (segs, saved) = {
            let sh = self.lock();
            (sh.st.segs.segs.clone(), sh.saved.clone())
        };
        let mut done = Vec::new();
        for s in &segs {
            let mut h = s.t0.div_euclid(HOUR_MS) * HOUR_MS;
            if h < s.t0 {
                h += HOUR_MS;
            }
            while h + HOUR_MS - 1 <= s.t1 {
                if !saved.contains(&h) {
                    let (mut ids, mut cols) = (Vec::new(), TradeCols::default());
                    for c in s.chunks.iter().filter(|c| {
                        c.cols.t_ms.last().is_some_and(|t| *t >= h) && c.cols.t_ms[0] < h + HOUR_MS
                    }) {
                        for i in 0..c.ids.len() {
                            let t = c.cols.t_ms[i];
                            if t >= h && t < h + HOUR_MS {
                                ids.push(c.ids[i]);
                                cols.t_ms.push(t);
                                cols.price.push(c.cols.price[i]);
                                cols.qty.push(c.cols.qty[i]);
                                cols.side.push(c.cols.side[i]);
                            }
                        }
                    }
                    if !ids.is_empty() {
                        let _ = write_bin(
                            &dir.join(format!("{}.bin", hour_str(h))),
                            FileKind::Hour,
                            Some(&ids),
                            &cols,
                        );
                    }
                    done.push(h);
                }
                h += HOUR_MS;
            }
        }
        self.lock().saved.extend(done);
    }

    /// `_evict`: segments ending more than 32 h ago, and day arrays no request can read any more.
    fn evict(&self) {
        let now = self.clock.now_ms();
        let old = now - ((MAX_REST_HOURS + 2) * HOUR_MS) as f64;
        let today = utc_day(now as i64);
        let mut sh = self.lock();
        sh.st.segs.segs.retain(|s| s.t1 as f64 >= old);
        sh.st.days.retain(|d, _| *d >= today - MAX_PAST_DAYS);
    }

    /// The window for `[t0, t1]` and the effects applied: floor, day downloads, loops started.
    fn window(self: &Arc<Self>, t0_ms: i64, t1_ms: i64) -> TradeWindow {
        let now = self.clock.now_ms();
        let (win, fx, start) = {
            let mut sh = self.lock();
            sh.last_used = now / 1000.0;
            let (win, fx) = assemble(&sh.st, t0_ms, t1_ms, now);
            if let Some(f) = fx.floor_ms {
                sh.st.floor_ms = Some(f);
            }
            for d in &fx.fetch_days {
                sh.st.day_busy.insert(*d);
            }
            let start = fx.start_loops && !sh.running;
            if start {
                sh.running = true;
            }
            (win, fx, start)
        };
        for d in fx.fetch_days {
            tokio::spawn(day_job(Arc::downgrade(self), d));
        }
        if start {
            tokio::spawn(supervise(Arc::downgrade(self)));
        }
        win
    }
}

/// `_day_job`: the cached day file, else the archive (parsed, then cached). A failure is retried
/// after 30 minutes.
async fn day_job(store: Weak<SymbolStore>, d: i64) {
    let Some(s) = store.upgrade() else { return };
    let cached = s
        .day_path(d)
        .and_then(|p| read_bin(&p).ok())
        .map(|(_, _, c)| c);
    let got = match cached {
        Some(c) => Ok(c),
        None => match s.src.daily_zip(&s.sym, &day_str(d)).await {
            Ok(bytes) => parse_daily_zip(&bytes).inspect(|c| {
                if let Some(p) = s.day_path(d) {
                    let _ = write_bin(&p, FileKind::Day, None, c);
                }
            }),
            Err(e) => Err(e),
        },
    };
    let now = s.clock.now_s();
    let mut sh = s.lock();
    match got {
        Ok(c) => {
            sh.st.days.insert(d, Arc::new(c));
        }
        Err(_) => {
            sh.st.day_fail.insert(d, now);
        }
    }
    sh.st.day_busy.remove(&d);
}

fn idle(s: &SymbolStore) -> bool {
    s.clock.now_s() - s.lock().last_used >= IDLE_STOP_S
}

/// `_run`: the head loop (its own task, never behind back-fill), back-fill, a 60 s save and evict,
/// and the idle stop. Restart and stop are decided under the store lock, so a request that arrives
/// during the idle exit starts the loops again instead of being lost.
async fn supervise(store: Weak<SymbolStore>) {
    let head = tokio::spawn(head_loop(store.clone()));
    let _abort_head = AbortOnDrop(head);
    let mut last_save = tokio::time::Instant::now();
    loop {
        let Some(s) = store.upgrade() else { return };
        if idle(&s) {
            let mut sh = s.lock();
            if s.clock.now_s() - sh.last_used >= IDLE_STOP_S {
                sh.running = false;
                drop(sh);
                s.save_hours();
                return;
            }
        }
        let worked = back_step(&s).await;
        if last_save.elapsed() > Duration::from_secs(60) {
            last_save = tokio::time::Instant::now();
            s.save_hours();
            s.evict();
        }
        drop(s);
        if !worked {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

async fn back_step(s: &SymbolStore) -> bool {
    let from = {
        let sh = s.lock();
        back_plan(&sh.st.segs, sh.st.floor_ms)
    };
    let Some(from_id) = from else { return false };
    match s
        .src
        .agg_trades(&s.sym, Some(from_id), Prio::Background)
        .await
    {
        Ok(Some(c)) => {
            let (t0, t1) = (
                c.cols.t_ms[0] + 1,
                *c.cols.t_ms.last().expect("a page has trades"),
            );
            s.lock().st.segs.add(c, t0, t1);
            true
        }
        Ok(None) => false,
        Err(_) => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            false
        }
    }
}

/// `_run_head`: the newest trades every `POLL_S`, following full pages at once.
async fn head_loop(store: Weak<SymbolStore>) {
    loop {
        let Some(s) = store.upgrade() else { return };
        if idle(&s) {
            return;
        }
        let started = tokio::time::Instant::now();
        loop {
            let t_req = s.clock.now_ms() - 1000.0;
            let plan = head_plan(&s.lock().st.segs, t_req);
            match s.src.agg_trades(&s.sym, plan.from_id, Prio::Chart).await {
                Ok(page) => {
                    if !apply_head(&mut s.lock().st.segs, plan, page, t_req) {
                        break;
                    }
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    break;
                }
            }
        }
        drop(s);
        let spent = started.elapsed().as_secs_f64();
        tokio::time::sleep(Duration::from_secs_f64(py_max(0.2, POLL_S - spent))).await;
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The per-symbol stores (`store(sym)`): created on first use, hours loaded then.
#[derive(Clone)]
pub struct Trades {
    src: Arc<dyn TradeSource>,
    clock: WallClock,
    cache: Option<PathBuf>,
    stores: Arc<Mutex<HashMap<String, Arc<SymbolStore>>>>,
}

impl Trades {
    pub fn new(src: Arc<dyn TradeSource>, clock: WallClock, cache: Option<PathBuf>) -> Trades {
        Trades {
            src,
            clock,
            cache,
            stores: Arc::default(),
        }
    }

    fn store(&self, sym: &str) -> Arc<SymbolStore> {
        let mut m = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        m.entry(sym.to_string())
            .or_insert_with(|| {
                Arc::new(SymbolStore {
                    sym: sym.to_string(),
                    src: self.src.clone(),
                    clock: self.clock,
                    cache: self.cache.clone(),
                    shared: Mutex::new(Shared {
                        st: StoreState::default(),
                        last_used: self.clock.now_s(),
                        running: false,
                        saved: HashSet::new(),
                    }),
                    loaded: tokio::sync::OnceCell::new(),
                })
            })
            .clone()
    }

    /// `store(sym).trades(t0, t1)`.
    pub async fn window(&self, sym: &str, t0_ms: i64, t1_ms: i64) -> TradeWindow {
        let s = self.store(sym);
        s.loaded.get_or_init(|| async { s.load_hours() }).await;
        s.window(t0_ms, t1_ms)
    }

    /// The store's state, for tests and the log panel.
    pub fn state(&self, sym: &str) -> Option<StoreState> {
        let m = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        m.get(sym).map(|s| s.lock().st.clone())
    }

    /// Whether the symbol's loops run.
    pub fn running(&self, sym: &str) -> bool {
        let m = self.stores.lock().unwrap_or_else(|e| e.into_inner());
        m.get(sym).is_some_and(|s| s.lock().running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(ids: std::ops::Range<i64>) -> Chunk {
        let n = ids.end - ids.start;
        Chunk {
            ids: ids.clone().collect(),
            cols: TradeCols {
                t_ms: ids.clone().map(|i| 1_000 + i * 10).collect(),
                price: (0..n).map(|i| 100.0 + i as f64).collect(),
                qty: vec![1.0; n as usize],
                side: (0..n).map(|i| if i % 2 == 0 { 1 } else { -1 }).collect(),
            },
        }
    }

    #[test]
    fn segments_join_is_order_independent_and_idempotent() {
        let parts = [(0..10), (10..25), (30..40), (5..15), (25..30)];
        let mut a = Segments::default();
        for p in parts.iter() {
            let c = chunk(p.clone());
            let (t0, t1) = (c.cols.t_ms[0], *c.cols.t_ms.last().unwrap());
            a.add(c, t0, t1);
        }
        let mut b = Segments::default();
        for p in parts.iter().rev().cloned().chain(parts.iter().cloned()) {
            let c = chunk(p.clone());
            let (t0, t1) = (c.cols.t_ms[0], *c.cols.t_ms.last().unwrap());
            b.add(c, t0, t1);
        }
        let flat = |s: &Segments| {
            s.segs
                .iter()
                .map(|g| {
                    (
                        g.first(),
                        g.last(),
                        g.t0,
                        g.t1,
                        g.chunks
                            .iter()
                            .flat_map(|c| c.ids.clone())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(flat(&a), flat(&b));
        assert_eq!(a.segs.len(), 1);
        assert_eq!(
            flat(&a)[0].4,
            (0..40).collect::<Vec<_>>(),
            "every id once, in order"
        );
    }

    #[test]
    fn codec_round_trip_and_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("of-codec-{}", std::process::id()));
        let c = chunk(0..5);
        let p = dir.join("x.bin");
        write_bin(&p, FileKind::Hour, Some(&c.ids), &c.cols).unwrap();
        let (k, ids, cols) = read_bin(&p).unwrap();
        assert_eq!(
            (k, ids.as_deref(), &cols),
            (FileKind::Hour, Some(c.ids.as_slice()), &c.cols)
        );
        std::fs::write(&p, b"OFTR garbage").unwrap();
        assert!(read_bin(&p).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(day_str(20_409), "2025-11-17");
        assert_eq!(hour_str(1_794_960_000_000 + 9 * HOUR_MS), "2026111809");
        assert_eq!(
            parse_hour("2026111809"),
            Some(1_794_960_000_000 + 9 * HOUR_MS)
        );
        assert_eq!(parse_hour("2026131809"), None);
    }

    #[test]
    fn plans_follow_python() {
        let mut s = Segments::default();
        assert_eq!(back_plan(&s, Some(0.0)), None);
        let c = chunk(5000..5100);
        let (t0, t1) = (c.cols.t_ms[0], *c.cols.t_ms.last().unwrap());
        s.add(c, t0, t1);
        assert_eq!(back_plan(&s, None), None, "no floor yet");
        assert_eq!(back_plan(&s, Some(0.0)), Some(4000), "below the oldest");
        assert_eq!(back_plan(&s, Some(t0 as f64)), None, "already at the floor");
        let c2 = chunk(6000..6050);
        let (u0, u1) = (c2.cols.t_ms[0], *c2.cols.t_ms.last().unwrap());
        s.add(c2, u0, u1);
        assert_eq!(back_plan(&s, Some(0.0)), Some(5100), "the newest gap first");
        let plan = head_plan(&s, u1 as f64 + 1000.0);
        assert_eq!(
            plan,
            HeadPlan {
                from_id: Some(6050),
                fresh_last: Some(6049)
            }
        );
        assert_eq!(
            head_plan(&s, u1 as f64 + 200_000.0).from_id,
            None,
            "stale head: newest page"
        );
    }
}
