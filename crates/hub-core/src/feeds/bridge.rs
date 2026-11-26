//! The bridge live-feed pollers (07-03, D-032c): one `/wait` long-poll task per
//! `(bridge, symbol, tf)` want, shared by every chart of that user that subscribes to it, and
//! stopped by the last unsubscribe. The poller pins what it vetted: the subscribe path resolved
//! the credentials and the addresses exactly once, like the REST path, so a later DNS answer
//! cannot move the connection. The broadcast frames are the same `ServerMsg::Bar` text the
//! Binance/Bybit/Deriv upstreams send.

use super::hub::frame;
use super::state::Frame;
use crate::bridges::client::{BridgeClient, BridgeError};
use crate::bridges::guard::Vetted;
use hub_types::market::{Bar, Tf};
use hub_types::ws::ServerMsg;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Which bridge a want is keyed on: the local-mode env bridge (`local`, D-033) or a named one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BridgeId {
    Local,
    Named(uuid::Uuid),
}

impl std::fmt::Display for BridgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeId::Local => f.write_str("mt5:local"),
            BridgeId::Named(id) => write!(f, "mt5:{id}"),
        }
    }
}

/// The want key (D-032c): the bridge's own `/wait` is keyed by symbol + tf, so tf joins the key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PollerKey {
    pub bridge: BridgeId,
    pub symbol: String,
    pub tf: Tf,
}

impl std::fmt::Display for PollerKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {}, {})", self.bridge, self.symbol, self.tf)
    }
}

struct PollerEntry {
    tx: broadcast::Sender<Frame>,
    task: Mutex<Option<JoinHandle<()>>>,
    subs: std::sync::atomic::AtomicUsize,
}

/// The registry [`AppState`](crate::AppState) carries (D-032c): one poller per want. `Clone`
/// shares the one map through the `Arc`.
#[derive(Clone, Default)]
pub struct BridgePollers {
    map: Arc<Mutex<HashMap<PollerKey, Arc<PollerEntry>>>>,
}

/// One viewer's bridge stream: the live `ServerMsg::Bar` frames the poller broadcasts.
/// Dropping it decrements the want's refcount; the last drop stops the poller.
pub struct BridgeSub {
    key: PollerKey,
    pollers: BridgePollers,
    rx: broadcast::Receiver<Frame>,
}

impl BridgeSub {
    /// The next frame, or `None` when the poller stopped. A slow viewer skips frames.
    pub async fn next(&mut self) -> Option<Frame> {
        loop {
            match self.rx.recv().await {
                Ok(f) => return Some(f),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl Drop for BridgeSub {
    fn drop(&mut self) {
        self.pollers.unsubscribe(&self.key);
    }
}

impl BridgePollers {
    /// Join the want's poller, starting it through `start` when this is the first viewer.
    /// `start` receives a clone of the entry's sender so the task can broadcast; its handle
    /// is what the last unsubscribe cancels.
    pub fn subscribe(
        &self,
        key: PollerKey,
        start: impl FnOnce(broadcast::Sender<Frame>) -> JoinHandle<()>,
    ) -> BridgeSub {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let entry = map
            .entry(key.clone())
            .or_insert_with(|| {
                Arc::new(PollerEntry {
                    tx: broadcast::channel(64).0,
                    task: Mutex::new(None),
                    subs: std::sync::atomic::AtomicUsize::new(0),
                })
            })
            .clone();
        entry.subs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut task = entry.task.lock().unwrap_or_else(|p| p.into_inner());
        if task.is_none() {
            *task = Some(start(entry.tx.clone()));
        }
        BridgeSub {
            key,
            pollers: self.clone(),
            rx: entry.tx.subscribe(),
        }
    }

    /// One viewer fewer; the last one stops the task and drops the entry.
    fn unsubscribe(&self, key: &PollerKey) {
        let mut map = self.map.lock().unwrap_or_else(|p| p.into_inner());
        let Some(entry) = map.get(key) else {
            return;
        };
        if entry.subs.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            if let Some(task) = entry.task.lock().unwrap_or_else(|p| p.into_inner()).take() {
                task.abort();
            }
            map.remove(key);
        }
    }
}

/// The poller loop: long-poll `/wait` for the want (the 25 s budget, one concurrency permit),
/// broadcast the changed candle as a `ServerMsg::Bar` frame, and stop on the error classes a
/// retry cannot fix (a wrong token, a refused address, a bad certificate) or after five
/// consecutive failures.
pub(crate) async fn run_poller(
    tx: broadcast::Sender<Frame>,
    client: Arc<BridgeClient>,
    bridge: uuid::Uuid,
    vetted: Vetted,
    token: String,
    symbol: String,
    tf: Tf,
) {
    let mut after = String::new();
    let mut misses = 0u32;
    loop {
        match client
            .wait(&bridge, &vetted, &token, &symbol, tf.as_str(), &after)
            .await
        {
            Ok(v) => {
                misses = 0;
                let key = v.get("key").and_then(|k| k.as_str()).unwrap_or("");
                if !key.is_empty() && key != after {
                    if let Some(bar) = v
                        .get("bar")
                        .and_then(|b| serde_json::from_value::<Bar>(b.clone()).ok())
                    {
                        let sent = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        let _ = tx.send(frame(&ServerMsg::Bar {
                            bar,
                            sent,
                            trade_ms: None,
                            lag_ms: None,
                        }));
                    }
                    after = key.to_string();
                }
            }
            // The bridge long-polls 20 s: silence inside the 25 s budget is the normal idle.
            Err(BridgeError::Timeout) => {}
            Err(e) => {
                misses += 1;
                tracing::warn!(target: "feed", src = "mt5", "{bridge} {symbol} {tf}: poller error: {e}");
                if matches!(
                    e,
                    BridgeError::Auth | BridgeError::Refused(_) | BridgeError::Cert
                ) || misses >= 5
                {
                    let _ = tx.send(frame(&ServerMsg::Status {
                        msg: format!("bridge poller stopped: {e}"),
                    }));
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn key() -> PollerKey {
        PollerKey {
            bridge: BridgeId::Named(uuid::Uuid::nil()),
            symbol: "XAUUSD".into(),
            tf: Tf::M1,
        }
    }

    #[tokio::test]
    async fn subscribe_shares_and_the_last_unsubscribe_stops() {
        let pollers = BridgePollers::default();
        let started = Arc::new(AtomicUsize::new(0));
        let spawn_noop = |started: &Arc<AtomicUsize>| {
            let started = started.clone();
            move |_| {
                started.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async {})
            }
        };
        let s1 = pollers.subscribe(key(), spawn_noop(&started));
        let s2 = pollers.subscribe(key(), spawn_noop(&started));
        assert_eq!(
            started.load(Ordering::SeqCst),
            1,
            "the second viewer shares the poller"
        );
        drop(s1);
        drop(s2);
        let s3 = pollers.subscribe(key(), spawn_noop(&started));
        assert_eq!(
            started.load(Ordering::SeqCst),
            2,
            "the last unsubscribe stopped the poller; a new subscribe starts a new one"
        );
        drop(s3);
    }

    #[test]
    fn the_wait_answer_maps_onto_a_bar_frame() {
        // The bridge's /wait answer (bridge/mt5_bridge.py:230) carries {bar, key, tick_ms};
        // the bar row maps onto Bar field for field and the frame carries it.
        let v: serde_json::Value = serde_json::json!({
            "bar": {"time": 1_700_000_000, "open": 1.0, "high": 2.0, "low": 0.5, "close": 1.5, "volume": 7.0},
            "key": "1700000000,1.5,2,0.5,7",
            "tick_ms": null
        });
        let bar: Bar = serde_json::from_value(v["bar"].clone()).expect("the row parses");
        assert_eq!(bar.time, 1_700_000_000, "time");
        assert_eq!(bar.open, 1.0, "open");
        assert_eq!(bar.high, 2.0, "high");
        assert_eq!(bar.low, 0.5, "low");
        assert_eq!(bar.close, 1.5, "close");
        assert_eq!(bar.volume, 7.0, "volume");
        let text = frame(&ServerMsg::Bar {
            bar,
            sent: 0,
            trade_ms: None,
            lag_ms: None,
        });
        assert!(
            text.contains("1700000000"),
            "the frame carries the bar: {text}"
        );
        assert!(text.contains("1.5"), "the close is in the frame");
    }
}
