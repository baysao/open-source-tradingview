//! Feed freshness and lag for `/api/health`, ported from `record_tick` / `health_report`
//! (`hub/server.py:534-552`). One instance per upstream; one writer (that upstream's ingest).

use hub_types::health::FeedHealthEntry;
use std::collections::VecDeque;

const WINDOW: usize = 300;

#[derive(Clone, Debug, Default)]
pub struct FeedHealth {
    ticks: u64,
    last_tick_ms: Option<i64>,
    lags: VecDeque<f64>,
}

impl FeedHealth {
    /// Records one upstream message received at `true_now_ms` for an event at `src_ms`; returns the
    /// lag in milliseconds (`None` when the event has no timestamp).
    pub fn record(&mut self, true_now_ms: f64, src_ms: i64) -> Option<f64> {
        self.ticks += 1;
        self.last_tick_ms = Some(true_now_ms as i64);
        if src_ms == 0 {
            return None;
        }
        let lag = true_now_ms - src_ms as f64;
        self.lags.push_back(lag);
        while self.lags.len() > WINDOW {
            self.lags.pop_front();
        }
        Some(lag)
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn last_tick_ms(&self) -> Option<i64> {
        self.last_tick_ms
    }

    pub fn lags(&self) -> impl Iterator<Item = f64> + '_ {
        self.lags.iter().copied()
    }

    /// The `/api/health` entry: median is `sorted[n // 2]`, p95 is `sorted[int(n * 0.95)]`.
    pub fn entry(&self, true_now_ms: i64) -> FeedHealthEntry {
        let mut sorted: Vec<f64> = self.lags.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pick = |i: usize| sorted.get(i).map(|v| v.round() as i64);
        let n = sorted.len();
        FeedHealthEntry {
            ticks: self.ticks,
            age_ms: true_now_ms - self.last_tick_ms.unwrap_or(0),
            lag_median_ms: if n == 0 { None } else { pick(n / 2) },
            lag_p95_ms: if n == 0 {
                None
            } else {
                pick(((n as f64) * 0.95) as usize)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_and_percentiles() {
        let mut h = FeedHealth::default();
        for i in 0..400 {
            h.record(10_000.0 + i as f64, 10_000);
        }
        let e = h.entry(10_500);
        assert_eq!(e.ticks, 400);
        assert_eq!(
            e.lag_median_ms,
            Some(250),
            "window keeps the last 300 lags (100..=399)"
        );
        assert_eq!(e.lag_p95_ms, Some(385));
        assert_eq!(e.age_ms, 101);
        assert_eq!(FeedHealth::default().entry(5).lag_median_ms, None);
    }
}
