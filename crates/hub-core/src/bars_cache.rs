//! The one way bars are fetched: an LRU of single-flight slots keyed by the normalized
//! `BarsQuery`, shared by `/api/bars` and (01-02) the websocket history, so a chart's HTTP call and
//! its websocket history right after it make one upstream call.
//!
//! TTL 5 s for the newest window (no `to`), 10 min for scroll-back pages. Errors are never cached.

use crate::market::{BarsQuery, FeedError};
use crate::sources::HistoryFetch;
use hub_types::market::{Bar, Source};
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::OnceCell;
// tokio's Instant equals std's at run time and follows the paused clock in tests.
use tokio::time::Instant;

pub const CAPACITY: usize = 512;
const TTL_NEWEST: Duration = Duration::from_secs(5);
const TTL_PAGE: Duration = Duration::from_secs(600);

/// `Cached` serves a live slot; `Fresh` (a new websocket series being seeded) refetches unless a
/// fetch for the same key is already in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    Cached,
    Fresh,
}

#[derive(Clone)]
struct Slot {
    cell: Arc<OnceCell<Arc<Vec<Bar>>>>,
    born: Instant,
}

/// Per-source counters with one writer each (the cache). `served` counts bars handed to callers
/// (Python's `bars_served`); `upstream_calls` counts fetches that reached the provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SourceStats {
    pub served: u64,
    pub upstream_calls: u64,
    pub errors: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServeStats {
    pub deriv: SourceStats,
    pub bybit: SourceStats,
    pub binance: SourceStats,
}

impl ServeStats {
    pub fn get(&self, s: Source) -> SourceStats {
        match s {
            Source::Deriv => self.deriv,
            Source::Bybit => self.bybit,
            Source::Binance => self.binance,
        }
    }
}

#[derive(Default)]
struct Counters {
    served: AtomicU64,
    upstream_calls: AtomicU64,
    errors: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> SourceStats {
        SourceStats {
            served: self.served.load(Ordering::Relaxed),
            upstream_calls: self.upstream_calls.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
pub struct BarsCache {
    inner: Arc<Inner>,
}

struct Inner {
    slots: Mutex<LruCache<BarsQuery, Slot>>,
    fetch: Arc<dyn HistoryFetch>,
    counters: [Counters; 3],
}

fn index(s: Source) -> usize {
    match s {
        Source::Deriv => 0,
        Source::Bybit => 1,
        Source::Binance => 2,
    }
}

impl BarsCache {
    pub fn new(fetch: Arc<dyn HistoryFetch>) -> BarsCache {
        BarsCache {
            inner: Arc::new(Inner {
                slots: Mutex::new(LruCache::new(
                    NonZeroUsize::new(CAPACITY).expect("capacity is non-zero"),
                )),
                fetch,
                counters: Default::default(),
            }),
        }
    }

    /// Bars for `q`, oldest first. Concurrent calls for one key share one upstream fetch.
    pub async fn get(
        &self,
        q: &BarsQuery,
        freshness: Freshness,
    ) -> Result<Arc<Vec<Bar>>, FeedError> {
        let ttl = if q.to_ms.is_some() {
            TTL_PAGE
        } else {
            TTL_NEWEST
        };
        let cell = {
            let mut slots = self.inner.slots.lock().unwrap_or_else(|p| p.into_inner());
            let reusable = slots.get(q).filter(|s| {
                let alive = s.born.elapsed() < ttl;
                let fresh_ok = freshness == Freshness::Cached || !s.cell.initialized();
                alive && fresh_ok
            });
            match reusable {
                Some(slot) => slot.cell.clone(),
                None => {
                    let slot = Slot {
                        cell: Arc::new(OnceCell::new()),
                        born: Instant::now(),
                    };
                    let cell = slot.cell.clone();
                    slots.put(q.clone(), slot);
                    cell
                }
            }
        };
        let counters = &self.inner.counters[index(q.source)];
        let fetch = self.inner.fetch.clone();
        let res = cell
            .get_or_try_init(|| async {
                counters.upstream_calls.fetch_add(1, Ordering::Relaxed);
                fetch.fetch(q).await.map(Arc::new)
            })
            .await
            .cloned();
        match &res {
            Ok(bars) => counters
                .served
                .fetch_add(bars.len() as u64, Ordering::Relaxed),
            Err(_) => counters.errors.fetch_add(1, Ordering::Relaxed),
        };
        res
    }

    /// Monotonic counters for the feed log (01-03).
    pub fn stats(&self) -> ServeStats {
        let c = &self.inner.counters;
        ServeStats {
            deriv: c[0].snapshot(),
            bybit: c[1].snapshot(),
            binance: c[2].snapshot(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::Ticker;
    use futures_util::future::BoxFuture;
    use hub_types::market::Tf;
    use std::sync::atomic::AtomicUsize;

    struct Counting {
        calls: AtomicUsize,
        fail: bool,
    }

    impl HistoryFetch for Counting {
        fn fetch<'a>(&'a self, _q: &'a BarsQuery) -> BoxFuture<'a, Result<Vec<Bar>, FeedError>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                if self.fail {
                    Err(FeedError::new("upstream down"))
                } else {
                    Ok(vec![Bar {
                        time: 60,
                        open: 1.0,
                        high: 1.0,
                        low: 1.0,
                        close: 1.0,
                        volume: 0.0,
                    }])
                }
            })
        }
    }

    fn query(to: Option<i64>) -> BarsQuery {
        BarsQuery::new(
            Source::Binance,
            Ticker::parse(Source::Binance, "BTCUSD").unwrap(),
            Tf::M1,
            1000,
            to,
        )
    }

    #[tokio::test(start_paused = true)]
    async fn single_flight_and_ttl() {
        let fake = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            fail: false,
        });
        let cache = BarsCache::new(fake.clone());
        let q = query(None);
        let gets = (0..8).map(|_| cache.get(&q, Freshness::Cached));
        let results = futures_util::future::join_all(gets).await;
        assert!(results.iter().all(Result::is_ok));
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            1,
            "eight concurrent gets share one fetch"
        );

        let page = query(Some(1_700_000_000_000));
        cache.get(&page, Freshness::Cached).await.unwrap();
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);

        tokio::time::advance(Duration::from_secs(6)).await;
        cache.get(&q, Freshness::Cached).await.unwrap();
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            3,
            "newest window expires after 5 s"
        );
        cache.get(&page, Freshness::Cached).await.unwrap();
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            3,
            "scroll-back page lives 10 min"
        );

        cache.get(&q, Freshness::Fresh).await.unwrap();
        assert_eq!(
            fake.calls.load(Ordering::SeqCst),
            4,
            "Fresh refetches a filled slot"
        );
        // twelve calls of one bar each were served; four reached the upstream
        assert_eq!(cache.stats().binance.served, 12);
        assert_eq!(cache.stats().binance.upstream_calls, 4);
    }

    #[tokio::test(start_paused = true)]
    async fn errors_are_not_cached() {
        let fake = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            fail: true,
        });
        let cache = BarsCache::new(fake.clone());
        let q = query(None);
        assert!(cache.get(&q, Freshness::Cached).await.is_err());
        assert!(cache.get(&q, Freshness::Cached).await.is_err());
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
        assert_eq!(cache.stats().binance.errors, 2);
    }
}
