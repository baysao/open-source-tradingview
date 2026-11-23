//! All feed state in one plain struct behind one `std::sync::Mutex` that is never held across an
//! `.await` (architecture base B). Every D-002 invariant is checked here, atomically, by
//! synchronous methods that take `now`:
//!
//! - at most `max_upstreams` distinct `(source, symbol)` upstreams; at the cap the longest-idle
//!   upstream in grace is evicted before a join is refused;
//! - at most `per_ip` sockets per client IP (loopback exempt while auth is off);
//! - an upstream lives while it has viewers or within `grace` of its last viewer leaving: the map
//!   entry's existence is the upstream's life (dropping it drops the stop sender, which ends the
//!   upstream task);
//! - one bar builder per `(upstream, timeframe)`, seeded once.

use crate::market::candles::{Applied, BarBuilder, FeedRules, Throttle, TickBatch};
use crate::market::feed_health::FeedHealth;
use crate::sources::UpstreamKey;
use axum::extract::ws::Utf8Bytes;
use hub_types::health::FeedHealthEntry;
use hub_types::market::{Bar, Source, Tf};
use hub_types::ws::ServerMsg;
use std::collections::{BTreeMap, HashMap};
use std::net::IpAddr;
use std::time::Duration;
use tokio::sync::{broadcast, oneshot};
use tokio::time::Instant;

/// One serialized server message, shared by reference with every viewer.
pub type Frame = Utf8Bytes;

/// Frames buffered per series before a slow viewer starts skipping (`Lagged`).
const SERIES_BUFFER: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_upstreams: usize,
    pub per_ip: u16,
    pub grace: Duration,
    /// Local mode (auth off): every tab is 127.0.0.1, so loopback is not counted (Q-ARCH-2).
    pub exempt_loopback: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_upstreams: 400,
            per_ip: 24,
            grace: Duration::from_secs(60),
            exempt_loopback: true,
        }
    }
}

/// Distinguishes two lives of the same key, so a leave or ingest from an evicted upstream is a
/// no-op.
pub type Epoch = u64;

/// Why a socket or subscription was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `FEED_MAX_SYMBOLS` upstreams are open and none is idle.
    Capacity,
    /// The 25th socket from one client IP.
    TooManySockets,
    /// History could not be fetched; the text is Python's (`Bybit history failed: ...`).
    History(String),
}

impl Refusal {
    /// The browser frame for a refusal that happens after the upgrade.
    pub fn message(&self) -> ServerMsg {
        match self {
            Refusal::Capacity => ServerMsg::Error {
                msg: "Too many markets are open on the server right now. Try again in a minute."
                    .into(),
                kind: Some(hub_types::ws::ErrorKind::Capacity),
            },
            Refusal::TooManySockets => ServerMsg::Error {
                msg: "too many chart connections from this address".into(),
                kind: None,
            },
            Refusal::History(msg) => ServerMsg::Error {
                msg: msg.clone(),
                kind: None,
            },
        }
    }
}

struct Series {
    builder: BarBuilder,
    throttle: Throttle,
    viewers: u32,
    tx: broadcast::Sender<Frame>,
}

/// Monotonic counters with one writer (this state); the feed log diffs them (01-03).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub updates: u64,
    pub ticks: u64,
    pub reconnects: u64,
    pub errors: u64,
}

struct Upstream {
    epoch: Epoch,
    rules: FeedRules,
    health: FeedHealth,
    idle_since: Option<Instant>,
    /// Dropped with the entry: the upstream task's `stop` receiver then resolves.
    _stop: oneshot::Sender<()>,
    series: BTreeMap<Tf, Series>,
    /// The current link notice (a `status` frame) while the link is down.
    last_notice: Option<Frame>,
    last_data: Option<Instant>,
    totals: Totals,
}

/// Returned by `join` when the upstream is new: the shell spawns the session after unlocking.
pub struct UpstreamStart {
    pub key: UpstreamKey,
    pub epoch: Epoch,
    pub stop: oneshot::Receiver<()>,
}

pub struct Joined {
    pub epoch: Epoch,
    pub rx: broadcast::Receiver<Frame>,
    pub start: Option<UpstreamStart>,
    /// The current link notice, sent to this viewer right after `history` (graft from A).
    pub notice: Option<Frame>,
}

/// One message to fan out, built under the lock and serialized after it.
pub struct Emit {
    pub tx: broadcast::Sender<Frame>,
    pub msg: ServerMsg,
}

/// A copy of what the feed log and `/api/health` read, taken under the lock.
#[derive(Clone, Debug, Default)]
pub struct FeedView {
    pub upstreams: Vec<UpstreamView>,
    pub totals: BTreeMap<Source, Totals>,
}

#[derive(Clone, Debug)]
pub struct UpstreamView {
    pub key: UpstreamKey,
    pub health: FeedHealthEntry,
    /// Lags in the window, when asked for (the feed log's 60 s summary).
    pub lags: Vec<f64>,
    pub series: Vec<(Tf, u32)>,
    pub idle: bool,
    pub last_data: Option<Instant>,
}

pub struct HubState {
    limits: Limits,
    upstreams: HashMap<UpstreamKey, Upstream>,
    sockets: HashMap<IpAddr, u16>,
    next_epoch: Epoch,
    /// Totals of upstreams that were removed, so per-source counters stay monotonic.
    retired: BTreeMap<Source, Totals>,
}

impl HubState {
    pub fn new(limits: Limits) -> HubState {
        HubState {
            limits,
            upstreams: HashMap::new(),
            sockets: HashMap::new(),
            next_epoch: 1,
            retired: BTreeMap::new(),
        }
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn upstream_count(&self) -> usize {
        self.upstreams.len()
    }

    /// Whether the `(key, tf)` series already has a seeded builder (decides `Fresh` history).
    pub fn has_series(&self, key: &UpstreamKey, tf: Tf) -> bool {
        self.upstreams
            .get(key)
            .is_some_and(|u| u.series.contains_key(&tf))
    }

    /// Counts a socket for `ip`; `Err` for the 25th. `exempt` skips counting (loopback, auth off).
    pub fn admit_socket(&mut self, ip: IpAddr, exempt: bool) -> Result<bool, Refusal> {
        if exempt && self.limits.exempt_loopback {
            return Ok(false);
        }
        let n = self.sockets.entry(ip).or_insert(0);
        if *n >= self.limits.per_ip {
            return Err(Refusal::TooManySockets);
        }
        *n += 1;
        Ok(true)
    }

    pub fn release_socket(&mut self, ip: IpAddr) {
        if let Some(n) = self.sockets.get_mut(&ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.sockets.remove(&ip);
            }
        }
    }

    /// Adds one viewer of `(key, tf)`. Creates the upstream when missing (evicting the longest-idle
    /// one at the cap), seeds the series once from `seed`, and subscribes the receiver under the same
    /// lock as the seed so no update falls between them.
    pub fn join(
        &mut self,
        key: &UpstreamKey,
        rules: FeedRules,
        tf: Tf,
        seed: Option<Bar>,
        now: Instant,
    ) -> Result<Joined, Refusal> {
        let mut start = None;
        if !self.upstreams.contains_key(key) {
            if self.upstreams.len() >= self.limits.max_upstreams {
                let idle = self
                    .upstreams
                    .iter()
                    .filter_map(|(k, u)| u.idle_since.map(|t| (t, k.clone())))
                    .min_by_key(|(t, _)| *t)
                    .map(|(_, k)| k);
                match idle {
                    Some(k) => self.remove(&k),
                    None => return Err(Refusal::Capacity),
                }
            }
            let epoch = self.next_epoch;
            self.next_epoch += 1;
            let (stop_tx, stop_rx) = oneshot::channel();
            self.upstreams.insert(
                key.clone(),
                Upstream {
                    epoch,
                    rules,
                    health: FeedHealth::default(),
                    idle_since: None,
                    _stop: stop_tx,
                    series: BTreeMap::new(),
                    last_notice: None,
                    // As Python (`last_data = time.time()` at stream start): a new upstream is not
                    // silent until SILENT_AFTER has passed without data.
                    last_data: Some(now),
                    totals: Totals::default(),
                },
            );
            start = Some(UpstreamStart {
                key: key.clone(),
                epoch,
                stop: stop_rx,
            });
        }
        let up = self.upstreams.get_mut(key).expect("inserted above");
        up.idle_since = None;
        let series = up.series.entry(tf).or_insert_with(|| Series {
            builder: BarBuilder::seed(tf, seed),
            throttle: Throttle::default(),
            viewers: 0,
            tx: broadcast::channel(SERIES_BUFFER).0,
        });
        series.viewers += 1;
        Ok(Joined {
            epoch: up.epoch,
            rx: series.tx.subscribe(),
            start,
            notice: up.last_notice.clone(),
        })
    }

    /// Removes one viewer. The series goes at zero viewers (a rejoin re-seeds it); the upstream
    /// enters grace when it has no series. A stale epoch is a no-op.
    pub fn leave(&mut self, key: &UpstreamKey, epoch: Epoch, tf: Tf, now: Instant) {
        let Some(up) = self.upstreams.get_mut(key).filter(|u| u.epoch == epoch) else {
            return;
        };
        if let Some(s) = up.series.get_mut(&tf) {
            s.viewers = s.viewers.saturating_sub(1);
            if s.viewers == 0 {
                up.series.remove(&tf);
            }
        }
        if up.series.is_empty() && up.idle_since.is_none() {
            up.idle_since = Some(now);
        }
    }

    /// Applies one upstream batch: one health sample, every tick to every series, the throttle,
    /// and on roll-over the closed bar first (D-004). Returns the frames to fan out.
    pub fn ingest(
        &mut self,
        key: &UpstreamKey,
        epoch: Epoch,
        batch: &TickBatch,
        now: Instant,
        true_now_ms: f64,
        local_now_ms: i64,
    ) -> Vec<Emit> {
        let Some(up) = self.upstreams.get_mut(key).filter(|u| u.epoch == epoch) else {
            return Vec::new();
        };
        let lag = up.health.record(true_now_ms, batch.src_ms);
        up.totals.ticks += 1;
        up.last_data = Some(now);
        up.last_notice = None;
        let trade_ms = up.rules.trade_ms_on_wire.then_some(batch.src_ms);
        let min_interval = up.rules.min_interval;
        let late = up.rules.late;
        let mut out = Vec::new();
        for series in up.series.values_mut() {
            let mut closed: Vec<Bar> = Vec::new();
            let mut changed = false;
            for t in &batch.ticks {
                match series.builder.apply(t, late) {
                    Applied::Updated => changed = true,
                    Applied::Rolled { closed: c } => {
                        closed.push(c);
                        changed = true;
                    }
                    Applied::Ignored => {}
                }
            }
            if !changed {
                continue;
            }
            let bar_msg = |bar: Bar| ServerMsg::Bar {
                bar,
                sent: local_now_ms,
                trade_ms,
                lag_ms: lag,
            };
            let rolled = !closed.is_empty();
            for c in closed {
                out.push(Emit {
                    tx: series.tx.clone(),
                    msg: bar_msg(c),
                });
            }
            let admit = if rolled {
                series.throttle.mark(now);
                true
            } else {
                series.throttle.admit(now, min_interval)
            };
            if admit {
                if let Some(cur) = series.builder.current() {
                    out.push(Emit {
                        tx: series.tx.clone(),
                        msg: bar_msg(cur),
                    });
                }
            }
        }
        up.totals.updates += out.len() as u64;
        out
    }

    /// The link went down (`Some(status)`) or ended with an error frame. Stores the notice for late
    /// joiners and returns the senders to broadcast it to.
    pub fn link_down(
        &mut self,
        key: &UpstreamKey,
        epoch: Epoch,
        notice: Frame,
        reconnect: bool,
    ) -> Vec<broadcast::Sender<Frame>> {
        let Some(up) = self.upstreams.get_mut(key).filter(|u| u.epoch == epoch) else {
            return Vec::new();
        };
        up.last_notice = Some(notice);
        if reconnect {
            up.totals.reconnects += 1;
        } else {
            up.totals.errors += 1;
        }
        up.series.values().map(|s| s.tx.clone()).collect()
    }

    /// Whether this upstream still exists with this epoch (the session loop's stop condition).
    pub fn alive(&self, key: &UpstreamKey, epoch: Epoch) -> bool {
        self.upstreams.get(key).is_some_and(|u| u.epoch == epoch)
    }

    /// Removes upstreams idle for at least `grace`. Returns the removed keys.
    pub fn sweep(&mut self, now: Instant) -> Vec<UpstreamKey> {
        let grace = self.limits.grace;
        let expired: Vec<UpstreamKey> = self
            .upstreams
            .iter()
            .filter(|(_, u)| u.idle_since.is_some_and(|t| now.duration_since(t) >= grace))
            .map(|(k, _)| k.clone())
            .collect();
        for k in &expired {
            self.remove(k);
        }
        expired
    }

    fn remove(&mut self, key: &UpstreamKey) {
        if let Some(up) = self.upstreams.remove(key) {
            let r = self.retired.entry(key.source).or_default();
            r.updates += up.totals.updates;
            r.ticks += up.totals.ticks;
            r.reconnects += up.totals.reconnects;
            r.errors += up.totals.errors;
        }
    }

    /// A copy for `/api/health` and the feed log; `with_lags` copies the lag windows too.
    pub fn view(&self, true_now_ms: i64, with_lags: bool) -> FeedView {
        let mut totals = self.retired.clone();
        let mut upstreams = Vec::with_capacity(self.upstreams.len());
        for (k, u) in &self.upstreams {
            let t = totals.entry(k.source).or_default();
            t.updates += u.totals.updates;
            t.ticks += u.totals.ticks;
            t.reconnects += u.totals.reconnects;
            t.errors += u.totals.errors;
            upstreams.push(UpstreamView {
                key: k.clone(),
                health: u.health.entry(true_now_ms),
                lags: if with_lags {
                    u.health.lags().collect()
                } else {
                    Vec::new()
                },
                series: u.series.iter().map(|(tf, s)| (*tf, s.viewers)).collect(),
                idle: u.idle_since.is_some(),
                last_data: u.last_data,
            });
        }
        upstreams.sort_by(|a, b| a.key.cmp(&b.key));
        FeedView { upstreams, totals }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::candles::Tick;
    use crate::market::Ticker;
    use crate::sources::binance;
    use std::net::Ipv4Addr;

    fn key(sym: &str) -> UpstreamKey {
        UpstreamKey {
            source: Source::Binance,
            symbol: Ticker::parse(Source::Binance, sym).unwrap(),
        }
    }

    fn bar(t: i64) -> Bar {
        Bar {
            time: t,
            open: 1.0,
            high: 1.0,
            low: 1.0,
            close: 1.0,
            volume: 1.0,
        }
    }

    fn limits(max: usize) -> Limits {
        Limits {
            max_upstreams: max,
            per_ip: 24,
            grace: Duration::from_secs(60),
            exempt_loopback: true,
        }
    }

    #[test]
    fn one_upstream_per_symbol_and_grace() {
        let t0 = Instant::now();
        let mut s = HubState::new(limits(400));
        let a = key("BTCUSD");
        let b = key("BTCUSDT");
        assert_eq!(a, b, "BTCUSD and BTCUSDT are one upstream");
        let mut starts = 0;
        let mut epochs = Vec::new();
        for i in 0..100 {
            let (k, tf) = if i % 2 == 0 {
                (&a, Tf::M1)
            } else {
                (&b, Tf::M5)
            };
            let j = s.join(k, binance::RULES, tf, Some(bar(0)), t0).unwrap();
            starts += j.start.is_some() as usize;
            epochs.push(j.epoch);
        }
        assert_eq!(starts, 1);
        assert_eq!(s.upstream_count(), 1);
        let e = epochs[0];
        for i in 0..100 {
            s.leave(
                &a,
                e,
                if i % 2 == 0 { Tf::M1 } else { Tf::M5 },
                t0 + Duration::from_secs(1),
            );
        }
        assert!(!s.has_series(&a, Tf::M1));
        s.sweep(t0 + Duration::from_secs(30));
        assert_eq!(s.upstream_count(), 1, "still in grace at 30 s");
        let _ = rejoin_within_grace_reuses_upstream_and_reseeds_series(&mut s, &a, e, t0);
        s.leave(&a, e, Tf::M1, t0 + Duration::from_secs(40));
        s.sweep(t0 + Duration::from_secs(99));
        assert_eq!(
            s.upstream_count(),
            1,
            "grace restarts at the last leave (40 s)"
        );
        assert_eq!(s.sweep(t0 + Duration::from_secs(101)), vec![a.clone()]);
        assert_eq!(s.upstream_count(), 0);
        s.leave(&a, e, Tf::M1, t0 + Duration::from_secs(102));
    }

    /// Called by the test above: a rejoin inside grace reuses the upstream and re-seeds the series.
    fn rejoin_within_grace_reuses_upstream_and_reseeds_series(
        s: &mut HubState,
        k: &UpstreamKey,
        e: Epoch,
        t0: Instant,
    ) -> Joined {
        let j = s
            .join(
                k,
                binance::RULES,
                Tf::M1,
                Some(bar(120)),
                t0 + Duration::from_secs(30),
            )
            .unwrap();
        assert!(j.start.is_none(), "same upstream");
        assert_eq!(j.epoch, e);
        assert!(s.has_series(k, Tf::M1));
        j
    }

    #[test]
    fn new_upstream_counts_as_fresh_until_data() {
        // Regression (01-03 browser check): a just-opened upstream logged "no data for 120 s".
        let t0 = Instant::now();
        let mut s = HubState::new(limits(400));
        s.join(&key("BTCUSD"), binance::RULES, Tf::M1, Some(bar(0)), t0)
            .unwrap();
        let v = s.view(0, false);
        assert_eq!(v.upstreams[0].last_data, Some(t0));
    }

    #[test]
    fn stale_epoch_is_a_no_op() {
        let t0 = Instant::now();
        let mut s = HubState::new(limits(1));
        let a = key("BTCUSD");
        let j = s.join(&a, binance::RULES, Tf::M1, None, t0).unwrap();
        s.leave(&a, j.epoch, Tf::M1, t0);
        let other = key("ETHUSD");
        s.join(&other, binance::RULES, Tf::M1, None, t0).unwrap();
        let j2 = s.join(&a, binance::RULES, Tf::M1, None, t0);
        assert_eq!(
            j2.err(),
            Some(Refusal::Capacity),
            "ETH is busy, nothing idle"
        );
        s.leave(&a, j.epoch, Tf::M1, t0);
        let batch = TickBatch {
            ticks: vec![Tick {
                price: 2.0,
                qty: 1.0,
                ts_ms: 60_000,
            }],
            src_ms: 60_000,
        };
        assert!(s
            .ingest(&a, j.epoch, &batch, t0, 60_100.0, 60_100)
            .is_empty());
    }

    #[test]
    fn capacity_evicts_the_longest_idle_upstream() {
        let t0 = Instant::now();
        let mut s = HubState::new(limits(2));
        let (a, b, c) = (key("BTCUSD"), key("ETHUSD"), key("SOLUSD"));
        let ja = s.join(&a, binance::RULES, Tf::M1, None, t0).unwrap();
        let jb = s.join(&b, binance::RULES, Tf::M1, None, t0).unwrap();
        assert!(s.join(&c, binance::RULES, Tf::M1, None, t0).is_err());
        s.leave(&a, ja.epoch, Tf::M1, t0 + Duration::from_secs(1));
        s.leave(&b, jb.epoch, Tf::M1, t0 + Duration::from_secs(2));
        assert!(s
            .join(
                &c,
                binance::RULES,
                Tf::M1,
                None,
                t0 + Duration::from_secs(3)
            )
            .is_ok());
        assert!(!s.alive(&a, ja.epoch), "the longest idle (BTC) was evicted");
        assert!(s.alive(&b, jb.epoch));
    }

    #[test]
    fn per_ip_limit_and_loopback_exemption() {
        let mut s = HubState::new(limits(10));
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 7));
        for _ in 0..24 {
            assert_eq!(s.admit_socket(ip, false), Ok(true));
        }
        assert_eq!(s.admit_socket(ip, false), Err(Refusal::TooManySockets));
        s.release_socket(ip);
        assert_eq!(s.admit_socket(ip, false), Ok(true));
        let lo = IpAddr::V4(Ipv4Addr::LOCALHOST);
        for _ in 0..100 {
            assert_eq!(
                s.admit_socket(lo, true),
                Ok(false),
                "loopback is not counted while auth is off"
            );
        }
    }

    #[test]
    fn ingest_throttles_and_publishes_closed_bar_on_roll() {
        let t0 = Instant::now();
        let mut s = HubState::new(limits(10));
        let a = key("BTCUSD");
        let j = s
            .join(
                &a,
                binance::RULES,
                Tf::M1,
                Some(Bar {
                    time: 0,
                    open: 1.0,
                    high: 1.0,
                    low: 1.0,
                    close: 1.0,
                    volume: 0.0,
                }),
                t0,
            )
            .unwrap();
        let tick = |p: f64, ms: i64| TickBatch {
            ticks: vec![Tick {
                price: p,
                qty: 1.0,
                ts_ms: ms,
            }],
            src_ms: ms,
        };
        assert_eq!(
            s.ingest(&a, j.epoch, &tick(2.0, 10_000), t0, 10_050.0, 10_050)
                .len(),
            1
        );
        assert_eq!(
            s.ingest(
                &a,
                j.epoch,
                &tick(3.0, 10_010),
                t0 + Duration::from_millis(10),
                10_060.0,
                10_060
            )
            .len(),
            0,
            "throttled"
        );
        let rolled = s.ingest(
            &a,
            j.epoch,
            &tick(4.0, 61_000),
            t0 + Duration::from_millis(20),
            61_050.0,
            61_050,
        );
        assert_eq!(
            rolled.len(),
            2,
            "closed bar first, then the new bar, despite the throttle"
        );
        match (&rolled[0].msg, &rolled[1].msg) {
            (
                ServerMsg::Bar {
                    bar: closed,
                    trade_ms,
                    lag_ms,
                    ..
                },
                ServerMsg::Bar { bar: new, .. },
            ) => {
                assert_eq!((closed.time, closed.close, closed.high), (0, 3.0, 3.0));
                assert_eq!((new.time, new.open), (60, 4.0));
                assert_eq!(*trade_ms, Some(61_000));
                assert_eq!(*lag_ms, Some(50.0));
            }
            other => panic!("unexpected {other:?}", other = other.0),
        }
        let v = s.view(61_100, false);
        assert_eq!(v.totals[&Source::Binance].updates, 3);
        assert_eq!(v.upstreams[0].health.ticks, 3);
    }
}
