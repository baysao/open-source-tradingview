//! Per-feed facts for the log panel, ported from `hub/feedlog.py`, counted once per upstream (not
//! per viewer socket, which multiplied every Python number by the viewer count).
//!
//! `FeedLog::step` is pure: it reads a `FeedView`, the status lights and the bars-cache counters,
//! keeps the previous totals itself, and returns lines plus the desired `feed.*` problem set:
//! - status edges every 30 s: `up: <msg>` / `DOWN: <msg>` once per change, problem `feed.<k>`;
//! - silence: a series with no upstream data for 120 s logs a warn once, problem
//!   `feed.<src>:<SYM TF>`; data flowing again logs info and clears it;
//! - a summary line per provider every 60 s in the Python format.

use crate::bars_cache::ServeStats;
use crate::feeds::state::{FeedView, Totals};
use crate::logbus::{Line, LogBus};
use hub_types::market::{Source, StatusResponse};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::time::Instant;

pub const SUMMARY_EVERY: Duration = Duration::from_secs(60);
pub const STATUS_EVERY: Duration = Duration::from_secs(30);
pub const SILENT_AFTER: Duration = Duration::from_secs(120);

pub struct FeedLogInput<'a> {
    pub now: Instant,
    pub feeds: &'a FeedView,
    pub status: &'a StatusResponse,
    pub bars: &'a ServeStats,
}

#[derive(Default)]
pub struct FeedLogOutput {
    pub lines: Vec<Line>,
    /// `(src, key, value, lvl, msg)` edge-triggered state lines.
    pub states: Vec<(String, String, String, String, String)>,
    /// The full desired set of `feed.*` problems.
    pub problems: BTreeMap<String, String>,
}

pub struct FeedLog {
    last_summary: Instant,
    /// First status check is 30 s after start, as Python (`last_status = time.time()`), so a
    /// catalog still loading at boot never logs `DOWN: 0 symbols`.
    last_status: Option<Instant>,
    prev_totals: BTreeMap<Source, Totals>,
    prev_bars: ServeStats,
    silent: BTreeSet<String>,
    status_problems: BTreeMap<String, String>,
}

fn label(symbol: &str, tf: &str) -> String {
    format!("{symbol} {tf}")
}

impl FeedLog {
    pub fn new(now: Instant) -> FeedLog {
        FeedLog {
            last_summary: now,
            last_status: Some(now),
            prev_totals: BTreeMap::new(),
            prev_bars: ServeStats::default(),
            silent: BTreeSet::new(),
            status_problems: BTreeMap::new(),
        }
    }

    /// Whether the next step writes a summary (so the caller copies lag windows only then).
    pub fn wants_lags(&self, now: Instant) -> bool {
        now.duration_since(self.last_summary) >= SUMMARY_EVERY
    }

    pub fn step(&mut self, input: FeedLogInput<'_>) -> FeedLogOutput {
        let mut out = FeedLogOutput::default();
        let now = input.now;

        if self
            .last_status
            .is_none_or(|t| now.duration_since(t) >= STATUS_EVERY)
        {
            self.last_status = Some(now);
            for (k, s) in [
                ("deriv", &input.status.deriv),
                ("bybit", &input.status.bybit),
                ("binance", &input.status.binance),
            ] {
                let tag = format!("feed.{k}");
                let (lvl, text) = if s.ok {
                    ("info", format!("up: {}", s.msg))
                } else {
                    ("warn", format!("DOWN: {}", s.msg))
                };
                out.states
                    .push((tag.clone(), "ok".into(), s.ok.to_string(), lvl.into(), text));
                if s.ok {
                    self.status_problems.remove(&tag);
                } else {
                    self.status_problems.insert(tag, s.msg.clone());
                }
            }
        }
        out.problems.extend(self.status_problems.clone());

        let mut still_silent = BTreeSet::new();
        for u in &input.feeds.upstreams {
            let tag = format!("feed.{}", u.key.source);
            let age = u.last_data.map(|t| now.duration_since(t));
            let silent = !u.series.is_empty() && age.is_none_or(|a| a > SILENT_AFTER);
            for (tf, _) in &u.series {
                let l = label(u.key.symbol.as_str(), tf.as_str());
                let key = format!("{tag}:{l}");
                if silent {
                    if !self.silent.contains(&key) {
                        let secs = age.map(|a| a.as_secs()).unwrap_or(SILENT_AFTER.as_secs());
                        out.lines.push(Line::new(
                            &tag,
                            "warn",
                            format!(
                                "{l}: no data for {secs} s (market closed, or the feed is dead)"
                            ),
                        ));
                    }
                    still_silent.insert(key.clone());
                    out.problems.insert(
                        key,
                        format!("no data for over {} s", SILENT_AFTER.as_secs()),
                    );
                } else if self.silent.contains(&key) {
                    out.lines
                        .push(Line::new(&tag, "info", format!("{l}: data flowing again")));
                }
            }
        }
        self.silent = still_silent;

        let elapsed = now.duration_since(self.last_summary);
        if elapsed >= SUMMARY_EVERY {
            let secs = elapsed.as_secs_f64();
            for src in Source::ALL {
                let tot = input.feeds.totals.get(&src).copied().unwrap_or_default();
                let prev = self.prev_totals.get(&src).copied().unwrap_or_default();
                let bars = input.bars.get(src);
                let prev_bars = self.prev_bars.get(src);
                let ups: Vec<_> = input
                    .feeds
                    .upstreams
                    .iter()
                    .filter(|u| u.key.source == src)
                    .collect();
                let series: Vec<String> = ups
                    .iter()
                    .flat_map(|u| {
                        u.series
                            .iter()
                            .map(move |(tf, _)| format!("{}:{}", u.key.symbol, tf))
                    })
                    .collect();
                let updates = tot.updates - prev.updates;
                let ticks = tot.ticks - prev.ticks;
                let reconnects = tot.reconnects - prev.reconnects;
                let errors = (tot.errors - prev.errors) + (bars.errors - prev_bars.errors);
                let served = bars.served - prev_bars.served;
                if series.is_empty()
                    && updates == 0
                    && ticks == 0
                    && reconnects == 0
                    && errors == 0
                    && served == 0
                {
                    continue;
                }
                let mut lags: Vec<f64> = ups.iter().flat_map(|u| u.lags.iter().copied()).collect();
                lags.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let lag = if lags.is_empty() {
                    "lag=n/a".to_string()
                } else {
                    format!(
                        "lag_med={}ms lag_p95={}ms",
                        lags[lags.len() / 2].round() as i64,
                        lags[((lags.len() as f64) * 0.95) as usize].round() as i64
                    )
                };
                let last = ups
                    .iter()
                    .filter(|u| !u.series.is_empty())
                    .filter_map(|u| u.last_data)
                    .max()
                    .map(|t| format!("last_data={}s", now.duration_since(t).as_secs()))
                    .unwrap_or_else(|| "last_data=n/a".to_string());
                let names = if series.is_empty() {
                    "none".to_string()
                } else {
                    series.join(",")
                };
                out.lines.push(Line::new(
                    format!("feed.{src}"),
                    "info",
                    format!(
                        "{} s: streams={} [{names}] updates={updates} ticks/s={:.1} {lag} bars_served={served} reconnects={reconnects} errors={errors} {last}",
                        secs as i64,
                        series.len(),
                        ticks as f64 / secs
                    ),
                ));
            }
            self.prev_totals = input.feeds.totals.clone();
            self.prev_bars = *input.bars;
            self.last_summary = now;
        }
        out
    }
}

/// Runs the feed log every 2 s and pushes into the bus.
pub fn spawn(
    bus: LogBus,
    feeds: crate::feeds::FeedHub,
    catalog: crate::catalog::Catalog,
    bars: crate::bars_cache::BarsCache,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut log = FeedLog::new(Instant::now());
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        loop {
            tick.tick().await;
            let now = Instant::now();
            let view = feeds.view(log.wants_lags(now));
            let out = log.step(FeedLogInput {
                now,
                feeds: &view,
                status: &catalog.status(),
                bars: &bars.stats(),
            });
            for (src, key, value, lvl, msg) in out.states {
                bus.state(&src, &key, &value, &lvl, &msg);
            }
            for line in out.lines {
                bus.push(line);
            }
            bus.replace_problems("feed.", out.problems);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feeds::state::UpstreamView;
    use crate::market::Ticker;
    use crate::sources::UpstreamKey;
    use hub_types::health::FeedHealthEntry;
    use hub_types::market::{FeedStatus, Tf};

    fn status(ok: bool) -> StatusResponse {
        let s = |ok: bool, m: &str| FeedStatus { ok, msg: m.into() };
        StatusResponse {
            deriv: s(ok, "43 symbols"),
            bybit: s(true, "3403 charts"),
            binance: s(true, "public feed"),
        }
    }

    fn view(updates: u64, ticks: u64, last_data: Option<Instant>) -> FeedView {
        let key = UpstreamKey {
            source: Source::Binance,
            symbol: Ticker::parse(Source::Binance, "BTCUSD").unwrap(),
        };
        FeedView {
            upstreams: vec![UpstreamView {
                key,
                health: FeedHealthEntry {
                    ticks,
                    age_ms: 0,
                    lag_median_ms: None,
                    lag_p95_ms: None,
                },
                lags: vec![10.0, 20.0, 30.0],
                series: vec![(Tf::M1, 5)],
                idle: false,
                last_data,
            }],
            totals: BTreeMap::from([(
                Source::Binance,
                Totals {
                    updates,
                    ticks,
                    reconnects: 0,
                    errors: 0,
                },
            )]),
        }
    }

    #[test]
    fn summary_counts_once_per_upstream() {
        let t0 = Instant::now();
        let mut log = FeedLog::new(t0);
        let bars = ServeStats::default();
        log.step(FeedLogInput {
            now: t0,
            feeds: &view(0, 0, Some(t0)),
            status: &status(true),
            bars: &bars,
        });
        let t60 = t0 + Duration::from_secs(60);
        let out = log.step(FeedLogInput {
            now: t60,
            feeds: &view(120, 300, Some(t60)),
            status: &status(true),
            bars: &bars,
        });
        let line = out
            .lines
            .iter()
            .find(|l| l.src == "feed.binance")
            .expect("binance summary");
        assert!(
            line.msg.starts_with(
                "60 s: streams=1 [BTCUSDT:M1] updates=120 ticks/s=5.0 lag_med=20ms lag_p95=30ms"
            ),
            "{}",
            line.msg
        );
        assert!(
            line.msg
                .contains("bars_served=0 reconnects=0 errors=0 last_data=0s"),
            "{}",
            line.msg
        );
    }

    #[test]
    fn silence_raises_and_clears_the_series_problem() {
        let t0 = Instant::now();
        let mut log = FeedLog::new(t0);
        let bars = ServeStats::default();
        let later = t0 + Duration::from_secs(121);
        let out = log.step(FeedLogInput {
            now: later,
            feeds: &view(0, 0, Some(t0)),
            status: &status(true),
            bars: &bars,
        });
        assert_eq!(
            out.problems
                .get("feed.binance:BTCUSDT M1")
                .map(String::as_str),
            Some("no data for over 120 s")
        );
        assert_eq!(
            out.lines
                .iter()
                .filter(|l| l.msg.contains("no data for 121 s"))
                .count(),
            1
        );
        let again = log.step(FeedLogInput {
            now: later + Duration::from_secs(2),
            feeds: &view(0, 0, Some(t0)),
            status: &status(true),
            bars: &bars,
        });
        assert!(
            again.lines.iter().all(|l| !l.msg.contains("no data for")),
            "logged once"
        );
        let back = log.step(FeedLogInput {
            now: later + Duration::from_secs(4),
            feeds: &view(1, 1, Some(later + Duration::from_secs(3))),
            status: &status(true),
            bars: &bars,
        });
        assert!(!back.problems.contains_key("feed.binance:BTCUSDT M1"));
        assert!(back
            .lines
            .iter()
            .any(|l| l.msg == "BTCUSDT M1: data flowing again"));
    }

    #[test]
    fn status_edges_and_problems() {
        let t0 = Instant::now();
        let mut log = FeedLog::new(t0);
        let bars = ServeStats::default();
        let boot = log.step(FeedLogInput {
            now: t0 + Duration::from_secs(2),
            feeds: &FeedView::default(),
            status: &status(false),
            bars: &bars,
        });
        assert!(
            boot.states.is_empty(),
            "no status line while the catalog loads"
        );
        assert!(boot.problems.is_empty());
        let t0 = t0 + Duration::from_secs(30);
        let out = log.step(FeedLogInput {
            now: t0,
            feeds: &FeedView::default(),
            status: &status(false),
            bars: &bars,
        });
        assert!(out
            .states
            .iter()
            .any(|s| s.0 == "feed.deriv" && s.4 == "DOWN: 43 symbols"));
        assert_eq!(
            out.problems.get("feed.deriv").map(String::as_str),
            Some("43 symbols")
        );
        let out = log.step(FeedLogInput {
            now: t0 + Duration::from_secs(30),
            feeds: &FeedView::default(),
            status: &status(true),
            bars: &bars,
        });
        assert!(out
            .states
            .iter()
            .any(|s| s.0 == "feed.deriv" && s.4 == "up: 43 symbols"));
        assert!(!out.problems.contains_key("feed.deriv"));
    }
}
