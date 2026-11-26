//! The live side: one shared poller per (symbol, timeframe), fanning snapshots to `/wait` callers.
//!
//! Ported from the oracle's `Poller`/`POLLERS` (`bridge/mt5_bridge.py` lines 188-245). One poller
//! per pair serves any number of charts and alerts, so the terminal — which is also the trading
//! path — sees a few reads a second regardless of how many callers wait.
//!
//! Each entry owns a [`watch::Sender`]; a waiter reads the current snapshot or awaits the next
//! change. A poller nobody asked for stops after `idle`, and its entry disappears.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use tokio::sync::watch;
use tokio::time::Instant;

use crate::term::{bar_key, last_tick, read_bars, Bar, Offset, Terminal, Tf};

/// How long a waiter may block before it answers with the current snapshot (Python's 20 s).
pub const WAIT_S: u64 = 20;
/// A poller nobody asked for stops after this long (the oracle's `POLLER_IDLE_S`).
pub const POLLER_IDLE_S: u64 = 60;
/// The live read's cadence (the oracle's `POLL_S`).
pub const POLL_MS: u64 = 250;

/// Everything `/wait` answers with.
#[derive(Clone, Debug, Default)]
pub struct LiveSnapshot {
    pub bar: Option<Bar>,
    pub key: String,
    pub tick_ms: Option<i64>,
    /// The last poll's failure, verbatim (the oracle raises it out of `wait()`).
    pub error: Option<String>,
    /// Set once the first poll finished, so a waiter can tell "nothing yet" from "quiet market".
    pub ready: bool,
}

/// The timing knobs. Tests shrink all of them; production uses the oracle's constants.
#[derive(Clone, Copy, Debug)]
pub struct LivePolicy {
    pub poll: StdDuration,
    pub idle: StdDuration,
    pub wait: StdDuration,
}

impl Default for LivePolicy {
    fn default() -> Self {
        Self {
            poll: StdDuration::from_millis(POLL_MS),
            idle: StdDuration::from_secs(POLLER_IDLE_S),
            wait: StdDuration::from_secs(WAIT_S),
        }
    }
}

type Entries = Arc<Mutex<HashMap<(String, Tf), Arc<Entry>>>>;

struct Entry {
    tx: watch::Sender<LiveSnapshot>,
    /// Unix ms of the last waiter touch or creation; the idle check compares against `idle`.
    asked: AtomicI64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The poller map: get-or-spawn per (symbol, timeframe).
pub struct PollerSet<M: Terminal> {
    live: Arc<M>,
    policy: LivePolicy,
    offset: watch::Receiver<Offset>,
    entries: Entries,
}

impl<M: Terminal + 'static> PollerSet<M> {
    /// Build the set over the LIVE session. `offset` is the shared single-writer offset channel.
    pub fn new(live: Arc<M>, offset: watch::Receiver<Offset>, policy: LivePolicy) -> Self {
        Self {
            live,
            policy,
            offset,
            entries: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// How many pollers run now (`/health` reports it).
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    /// One long-poll: return once the key differs from `after`, or at the deadline.
    ///
    /// The oracle's loop: wait while (`not ready` or `key == after`) and no error, re-touching
    /// `asked` on every wake so a caller keeps its poller alive; then raise the error, if any.
    pub async fn wait(&self, sym: &str, tf: Tf, after: &str) -> Result<LiveSnapshot, String> {
        let entry = self.entry(sym, tf);
        entry.asked.store(now_ms(), Ordering::SeqCst);
        let mut rx = entry.tx.subscribe();
        let end = Instant::now() + self.policy.wait;
        loop {
            {
                let snap = rx.borrow_and_update().clone();
                if let Some(err) = snap.error.clone() {
                    return Err(err);
                }
                if snap.ready && snap.key != after {
                    return Ok(snap);
                }
            }
            let now = Instant::now();
            let Some(left) = end.checked_duration_since(now) else {
                let snap = rx.borrow().clone();
                return match snap.error.clone() {
                    Some(err) => Err(err),
                    None => Ok(snap),
                };
            };
            // The oracle wakes at least once a second to re-touch its idle clock.
            entry.asked.store(now_ms(), Ordering::SeqCst);
            let _ = tokio::time::timeout(left.min(StdDuration::from_secs(1)), rx.changed()).await;
        }
    }

    fn entry(&self, sym: &str, tf: Tf) -> Arc<Entry> {
        let mut map = self.entries.lock().unwrap();
        let key = (sym.to_string(), tf);
        if let Some(e) = map.get(&key) {
            return e.clone();
        }
        let (tx, _rx) = watch::channel(LiveSnapshot::default());
        let entry = Arc::new(Entry {
            tx,
            asked: AtomicI64::new(now_ms()),
        });
        map.insert(key.clone(), entry.clone());
        let runner = self.runner(key, entry.clone());
        tokio::spawn(runner);
        entry
    }

    /// The poller task: a few reads a second while somebody waits, then it removes its own entry.
    fn runner(
        &self,
        key: (String, Tf),
        entry: Arc<Entry>,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        let live = self.live.clone();
        let policy = self.policy;
        let offset = self.offset.clone();
        let entries = self.entries.clone();
        let (sym, tf) = key.clone();
        async move {
            let mut first = true;
            loop {
                if now_ms() - entry.asked.load(Ordering::SeqCst) >= policy.idle.as_millis() as i64 {
                    break;
                }
                let off = offset.borrow().offset_s;
                // A short window keeps each read tiny; the first read reaches further back so a
                // shut market still answers (the oracle's `max(sec*4, 10*86400)`).
                let span = if first {
                    (tf.sec() * 4).max(10 * 86400)
                } else {
                    tf.sec() * 4 + 86400
                };
                let got = read_bars(&*live, &sym, tf, now_ms() / 1000 + 86400, span, off).await;
                let mut next = entry.tx.borrow().clone();
                match got {
                    Ok(rows) => {
                        next.error = None;
                        next.ready = true;
                        if let Ok(Some(ms)) = last_tick(&*live, &sym, off).await {
                            next.tick_ms = Some(ms);
                        }
                        if let Some(last) = rows.last() {
                            let k = bar_key(Some(last));
                            if k != next.key {
                                next.bar = Some(last.clone());
                                next.key = k;
                            }
                        }
                        entry.tx.send_replace(next);
                    }
                    Err(e) => {
                        next.error = Some(e.to_string());
                        next.ready = true;
                        entry.tx.send_replace(next);
                    }
                }
                first = false;
                tokio::time::sleep(policy.poll).await;
            }
            let mut map = entries.lock().unwrap();
            if map.get(&key).is_some_and(|e| Arc::ptr_eq(e, &entry)) {
                map.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    use serde_json::{json, Value};

    /// A terminal whose newest bar advances every `advance_every` calls; `fail` flips it to
    /// answering errors, which is how AC-4 drives a wedged poller.
    struct Fs {
        calls: AtomicUsize,
        fail: std::sync::atomic::AtomicBool,
        advance_every: usize,
    }

    impl Fs {
        fn new(advance_every: usize) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                fail: std::sync::atomic::AtomicBool::new(false),
                advance_every,
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl Terminal for Fs {
        fn call(
            &self,
            name: &str,
            _args: Value,
        ) -> impl std::future::Future<Output = Result<Value, crate::mcp::McpError>> + Send {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let fail = self.fail.load(Ordering::SeqCst);
            // `checked_div` keeps the "never advance" case (divisor 0) lint-clean.
            let step = (n as i64)
                .checked_div(self.advance_every as i64)
                .unwrap_or(0);
            async move {
                if fail {
                    return Err(crate::mcp::McpError::Transport("terminal is down".into()));
                }
                match name {
                    "get_chart_history" => Ok(json!({"history": [{
                        "time": crate::term::iso(1_764_150_000 + step * 60),
                        "open": 1.0, "high": 2.0, "low": 0.5, "close": 1.5 + step as f64,
                        "tick_volume": 7.0
                    }]})),
                    "get_chart_ticks_history" => Ok(json!({"history": [{
                        "time_ms": crate::term::iso(1_764_150_000)
                    }]})),
                    other => Err(crate::mcp::McpError::Terminal(format!("no tool {other}"))),
                }
            }
        }
    }

    fn policy() -> LivePolicy {
        LivePolicy {
            poll: StdDuration::from_millis(10),
            idle: StdDuration::from_millis(300),
            wait: StdDuration::from_millis(400),
        }
    }

    fn set(fs: Arc<Fs>) -> PollerSet<Fs> {
        let (_tx, rx) = watch::channel(Offset::default());
        // The sender must outlive the set for the receiver to stay open.
        std::mem::forget(_tx);
        PollerSet::new(fs, rx, policy())
    }

    #[tokio::test]
    async fn one_poller_serves_every_waiter() {
        let fs = Fs::new(1_000_000); // the bar never changes on its own
        let set = Arc::new(set(fs.clone()));
        assert_eq!(set.len(), 0);
        let first = set.wait("XAUUSDm", Tf::M1, "").await.expect("seed");
        // The second waiter holds the current key: it stays parked on the poller until the
        // deadline, so the poller window is long enough to count calls.
        let started = Instant::now();
        let _second = set
            .wait("XAUUSDm", Tf::M1, &first.key)
            .await
            .expect("deadline answer");
        assert!(started.elapsed() >= policy().idle.min(policy().wait));
        assert_eq!(set.len(), 1, "one entry for the pair, not two");
        // One poller over that window: the bar is static (advance 1 in a million calls), so a
        // second spawned poller would roughly double the terminal calls below.
        let calls = fs.calls();
        assert!(
            calls < 60,
            "one shared poller made {calls} calls over the parked window"
        );
    }

    #[tokio::test]
    async fn a_waiter_on_the_current_key_blocks_until_it_changes() {
        // A new bar on every poll: the waiter holding the old key must see the change.
        let fs = Fs::new(1);
        let set = Arc::new(set(fs.clone()));
        let first = set.wait("EURUSD", Tf::M1, "").await.expect("first");
        let second = set
            .wait("EURUSD", Tf::M1, &first.key)
            .await
            .expect("second");
        assert_ne!(second.key, first.key, "the key moved");
        assert_eq!(
            bar_key(second.bar.as_ref()),
            second.key,
            "the returned bar and key agree"
        );
    }

    #[tokio::test]
    async fn a_quiet_market_returns_at_the_deadline() {
        let fs = Fs::new(1_000_000);
        let set = Arc::new(set(fs.clone()));
        let first = set.wait("EURUSD", Tf::M1, "").await.expect("first");
        let started = Instant::now();
        let again = set
            .wait("EURUSD", Tf::M1, &first.key)
            .await
            .expect("deadline answer");
        assert_eq!(again.key, first.key, "no change, same snapshot");
        assert!(
            started.elapsed() >= policy().wait,
            "held to the deadline, not returned early"
        );
    }

    #[tokio::test]
    async fn an_idle_poller_stops_and_leaves_the_map() {
        let fs = Fs::new(1_000_000);
        let set = Arc::new(set(fs.clone()));
        let _ = set.wait("EURUSD", Tf::M1, "").await.expect("first");
        assert_eq!(set.len(), 1);
        tokio::time::sleep(StdDuration::from_millis(700)).await;
        assert_eq!(set.len(), 0, "the entry disappeared after the idle window");
        let quiet = fs.calls();
        tokio::time::sleep(StdDuration::from_millis(100)).await;
        assert_eq!(
            fs.calls(),
            quiet,
            "no terminal call after the poller stopped"
        );
    }

    #[tokio::test]
    async fn a_terminal_error_reaches_the_waiter_and_does_not_wedge_it() {
        let fs = Fs::new(1_000_000);
        fs.fail.store(true, Ordering::SeqCst);
        let set = Arc::new(set(fs.clone()));
        let err = set
            .wait("EURUSD", Tf::M1, "")
            .await
            .expect_err("a failing terminal surfaces as an error");
        assert_eq!(err, "terminal is down");

        fs.fail.store(false, Ordering::SeqCst);
        // The next successful poll clears the snapshot's error; give it a poll interval.
        tokio::time::sleep(StdDuration::from_millis(40)).await;
        let later = set
            .wait("EURUSD", Tf::M1, "")
            .await
            .expect("the next wait succeeds again");
        assert!(later.error.is_none());
    }
}
