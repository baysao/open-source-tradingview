//! The thin async shell around `HubState`: subscribe, socket permits, the sweeper, health.

use super::state::{Frame, HubState, Limits, Refusal};
use super::upstream;
use crate::bars_cache::{BarsCache, Freshness};
use crate::clock::Clock;
use crate::market::{BarsQuery, Ticker};
use crate::sources::{self, Endpoints, UpstreamKey};
use hub_types::health::HealthResponse;
use hub_types::market::{Source, Tf};
use hub_types::ws::ServerMsg;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::Instant;

/// What a browser asked for, already parsed (source fallback applied, tf validated).
#[derive(Clone, Debug)]
pub struct Wanted {
    pub source: Source,
    pub symbol: String,
    pub tf: Tf,
    /// 07-03 (D-032a): the `bridge` field, verbatim. `None` is every pre-07-03 subscribe: the
    /// Binance path bit for bit. The string is validated at subscribe time, where the state
    /// can check it (a uuid the session owns, or `local` when auth is off), not in the pure
    /// parser.
    pub bridge: Option<String>,
}

#[derive(Clone)]
pub struct FeedHub {
    inner: Arc<Inner>,
}

struct Inner {
    state: Mutex<HubState>,
    bars: BarsCache,
    clock: Clock,
    endpoints: Endpoints,
}

/// Serializes once; every viewer gets a reference-counted copy.
pub fn frame(msg: &ServerMsg) -> Frame {
    Frame::from(
        serde_json::to_string(msg)
            .unwrap_or_else(|_| r#"{"type":"error","msg":"serialize"}"#.to_string()),
    )
}

impl FeedHub {
    pub fn new(limits: Limits, bars: BarsCache, clock: Clock, endpoints: Endpoints) -> FeedHub {
        FeedHub {
            inner: Arc::new(Inner {
                state: Mutex::new(HubState::new(limits)),
                bars,
                clock,
                endpoints,
            }),
        }
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, HubState> {
        self.inner.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn clock(&self) -> &Clock {
        &self.inner.clock
    }

    /// Counts a socket for the client IP (RAII). `loopback_peer` exempts it while auth is off.
    pub fn admit_socket(&self, ip: IpAddr, loopback_peer: bool) -> Result<SocketPermit, Refusal> {
        let counted = self.lock().admit_socket(ip, loopback_peer)?;
        Ok(SocketPermit {
            hub: self.clone(),
            ip: counted.then_some(ip),
        })
    }

    /// History, capacity, join and seed in one call. History is fetched before the join, so a
    /// symbol whose history fails never takes a capacity slot.
    pub async fn subscribe(&self, want: Wanted) -> Result<Subscription, Refusal> {
        let source = want.source;
        let history_err =
            |e: String| Refusal::History(format!("{} history failed: {e}", source.display()));
        let ticker = Ticker::parse(source, &want.symbol).map_err(|e| history_err(e.0))?;
        let key = UpstreamKey {
            source,
            symbol: ticker.clone(),
        };
        let fresh = if self.lock().has_series(&key, want.tf) {
            Freshness::Cached
        } else {
            Freshness::Fresh
        };
        let q = BarsQuery::new(source, ticker.clone(), want.tf, 1000, None);
        let bars = self
            .inner
            .bars
            .get(&q, fresh)
            .await
            .map_err(|e| history_err(e.0))?;
        let plan = sources::live_plan(&key, &self.inner.endpoints);
        let joined = self.lock().join(
            &key,
            plan.rules,
            want.tf,
            bars.last().copied(),
            Instant::now(),
        )?;
        if let Some(start) = joined.start {
            let hub = self.clone();
            tokio::spawn(upstream::run(hub, start, plan));
            tracing::info!(target: "feed", src = %source, "{key}: upstream opened");
        }
        // Binance reports the mapped symbol, Bybit and Deriv echo the request (Python parity).
        let symbol = match source {
            Source::Binance => ticker.as_str().to_string(),
            Source::Bybit | Source::Deriv => want.symbol.clone(),
        };
        let history = frame(&ServerMsg::History {
            bars: bars.to_vec(),
            source: source.as_str().to_string(),
            symbol,
            tf: want.tf.as_str().to_string(),
        });
        Ok(Subscription {
            hub: self.clone(),
            key,
            epoch: joined.epoch,
            tf: want.tf,
            rx: joined.rx,
            history: Some(history),
            notice: joined.notice,
        })
    }

    /// `/api/health`.
    pub fn health(&self) -> HealthResponse {
        let now = self.inner.clock.true_now_ms() as i64;
        let view = self.lock().view(now, false);
        HealthResponse {
            clock_offset_ms: self.inner.clock.offset_ms(),
            feeds: view
                .upstreams
                .into_iter()
                .map(|u| (u.key.to_string(), u.health))
                .collect(),
        }
    }

    /// A copy of the feed state for the feed log (01-03).
    pub fn view(&self, with_lags: bool) -> super::state::FeedView {
        let now = self.inner.clock.true_now_ms() as i64;
        self.lock().view(now, with_lags)
    }

    /// Removes upstreams whose grace ran out, every second.
    pub fn spawn_sweeper(&self) -> tokio::task::JoinHandle<()> {
        let hub = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                for k in hub.lock().sweep(Instant::now()) {
                    tracing::info!(target: "feed", src = %k.source, "{k}: upstream closed after 60 s without viewers");
                }
            }
        })
    }
}

/// Proof that a socket was admitted; dropping it frees the per-IP slot.
pub struct SocketPermit {
    hub: FeedHub,
    ip: Option<IpAddr>,
}

impl Drop for SocketPermit {
    fn drop(&mut self) {
        if let Some(ip) = self.ip {
            self.hub.lock().release_socket(ip);
        }
    }
}

/// One viewer's stream: `history` first, the link notice if the link is down, then live frames.
/// Dropping it leaves the series.
pub struct Subscription {
    hub: FeedHub,
    key: UpstreamKey,
    epoch: u64,
    tf: Tf,
    rx: broadcast::Receiver<Frame>,
    history: Option<Frame>,
    notice: Option<Frame>,
}

impl Subscription {
    /// The next frame, or `None` when the upstream is gone. A slow viewer skips frames.
    pub async fn next(&mut self) -> Option<Frame> {
        if let Some(h) = self.history.take() {
            return Some(h);
        }
        if let Some(n) = self.notice.take() {
            return Some(n);
        }
        loop {
            match self.rx.recv().await {
                Ok(f) => return Some(f),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub
            .lock()
            .leave(&self.key, self.epoch, self.tf, Instant::now());
    }
}
