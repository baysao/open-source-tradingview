//! Live bar building, ported from the three Python stream loops (`hub/server.py:329-367`,
//! `453-512`, `651-684`). Pure: the caller passes every time value.
//!
//! A bar closes only when the first trade of the next bucket arrives (no timer). Late trades merge
//! into the current bar for Binance and Deriv and are dropped for Bybit. On roll-over the closed
//! bar's final state is published once before the new bar (D-004).

use hub_types::market::{Bar, Tf};
use std::time::Duration;
use tokio::time::Instant;

/// One trade or tick, normalized by a provider parser.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick {
    pub price: f64,
    /// Base quantity; 0.0 for Deriv ticks and for Bybit mark and index prices.
    pub qty: f64,
    /// Upstream event time, epoch milliseconds.
    pub ts_ms: i64,
}

/// One upstream message: the ticks it carried and the timestamp used for lag and `trade_ms`
/// (Binance: the trade time; Bybit: the last trade's time or the ticker's `ts`; Deriv: the tick
/// epoch). One batch is one health sample, which keeps Python's `record_tick` granularity.
#[derive(Clone, Debug, PartialEq)]
pub struct TickBatch {
    pub ticks: Vec<Tick>,
    pub src_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatePolicy {
    /// An older trade updates the current bar (Binance, Deriv).
    Merge,
    /// An older trade is ignored (Bybit).
    Drop,
}

/// Per-provider rules, read by the builder and the fan-out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeedRules {
    pub late: LatePolicy,
    /// Binance and Bybit: at most one frame per 50 ms per series. Deriv: every tick.
    pub min_interval: Option<Duration>,
    /// Binance and Bybit send `trade_ms`; Deriv does not.
    pub trade_ms_on_wire: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Applied {
    Updated,
    /// The tick opened a new bucket; `closed` is the previous bar's final state.
    Rolled {
        closed: Bar,
    },
    Ignored,
}

/// The forming bar of one `(upstream, timeframe)` series.
#[derive(Clone, Debug, PartialEq)]
pub struct BarBuilder {
    span: i64,
    cur: Option<Bar>,
}

impl BarBuilder {
    /// Seeded from the last history bar (the forming one); `None` opens on the first tick.
    pub fn seed(tf: Tf, last: Option<Bar>) -> BarBuilder {
        BarBuilder {
            span: tf.secs(),
            cur: last,
        }
    }

    pub fn current(&self) -> Option<Bar> {
        self.cur
    }

    pub fn apply(&mut self, t: &Tick, late: LatePolicy) -> Applied {
        let ts = t.ts_ms.div_euclid(1000);
        let start = ts - ts.rem_euclid(self.span);
        let fresh = Bar {
            time: start,
            open: t.price,
            high: t.price,
            low: t.price,
            close: t.price,
            volume: t.qty,
        };
        match self.cur {
            None => {
                self.cur = Some(fresh);
                Applied::Updated
            }
            Some(cur) if start > cur.time => {
                self.cur = Some(fresh);
                Applied::Rolled { closed: cur }
            }
            Some(cur) if start < cur.time && late == LatePolicy::Drop => {
                let _ = cur;
                Applied::Ignored
            }
            Some(mut cur) => {
                cur.high = cur.high.max(t.price);
                cur.low = cur.low.min(t.price);
                cur.close = t.price;
                cur.volume += t.qty;
                self.cur = Some(cur);
                Applied::Updated
            }
        }
    }
}

/// At most one frame per `min_interval`; `None` admits everything.
#[derive(Clone, Debug, Default)]
pub struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    pub fn admit(&mut self, now: Instant, min_interval: Option<Duration>) -> bool {
        let Some(gap) = min_interval else { return true };
        match self.last {
            Some(last) if now.duration_since(last) < gap => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }

    /// Marks a frame sent now (the roll-over path bypasses `admit` but still resets the window).
    pub fn mark(&mut self, now: Instant) {
        self.last = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(price: f64, qty: f64, ts_s: i64) -> Tick {
        Tick {
            price,
            qty,
            ts_ms: ts_s * 1000 + 250,
        }
    }

    fn seed() -> Bar {
        Bar {
            time: 600,
            open: 10.0,
            high: 11.0,
            low: 9.0,
            close: 10.5,
            volume: 2.0,
        }
    }

    #[test]
    fn update_merges_into_the_forming_bar() {
        let mut b = BarBuilder::seed(Tf::M1, Some(seed()));
        assert_eq!(
            b.apply(&tick(12.0, 1.0, 630), LatePolicy::Merge),
            Applied::Updated
        );
        assert_eq!(
            b.current().unwrap(),
            Bar {
                time: 600,
                open: 10.0,
                high: 12.0,
                low: 9.0,
                close: 12.0,
                volume: 3.0
            }
        );
    }

    #[test]
    fn roll_over_reports_the_closed_bar() {
        let mut b = BarBuilder::seed(Tf::M1, Some(seed()));
        b.apply(&tick(12.0, 1.0, 630), LatePolicy::Merge);
        let r = b.apply(&tick(13.0, 0.5, 661), LatePolicy::Merge);
        assert_eq!(
            r,
            Applied::Rolled {
                closed: Bar {
                    time: 600,
                    open: 10.0,
                    high: 12.0,
                    low: 9.0,
                    close: 12.0,
                    volume: 3.0
                }
            }
        );
        assert_eq!(
            b.current().unwrap(),
            Bar {
                time: 660,
                open: 13.0,
                high: 13.0,
                low: 13.0,
                close: 13.0,
                volume: 0.5
            }
        );
    }

    #[test]
    fn late_trade_merges_for_binance_and_deriv_but_not_bybit() {
        let late = tick(1.0, 4.0, 590);
        let mut merge = BarBuilder::seed(Tf::M1, Some(seed()));
        assert_eq!(merge.apply(&late, LatePolicy::Merge), Applied::Updated);
        assert_eq!(merge.current().unwrap().low, 1.0);
        let mut drop = BarBuilder::seed(Tf::M1, Some(seed()));
        assert_eq!(drop.apply(&late, LatePolicy::Drop), Applied::Ignored);
        assert_eq!(drop.current().unwrap(), seed());
    }

    #[test]
    fn empty_history_opens_on_first_tick_and_d1_is_utc_midnight() {
        let mut b = BarBuilder::seed(Tf::D1, None);
        b.apply(&tick(5.0, 0.0, 86_400 * 3 + 5_000), LatePolicy::Merge);
        assert_eq!(b.current().unwrap().time, 86_400 * 3);
    }

    #[test]
    fn throttle_admits_once_per_window() {
        let t0 = Instant::now();
        let mut th = Throttle::default();
        let gap = Some(Duration::from_millis(50));
        assert!(th.admit(t0, gap));
        assert!(!th.admit(t0 + Duration::from_millis(30), gap));
        assert!(th.admit(t0 + Duration::from_millis(50), gap));
        assert!(th.admit(t0, None));
    }
}
