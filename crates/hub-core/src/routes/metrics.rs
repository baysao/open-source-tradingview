//! TTFC, defined once (C16): "time to first live candle: navigation start to the first live bar
//! update rendered on a chart engine page".
//!
//! The page measures it (`hub/ttfc.js`) and posts it once per load to `POST /api/metrics/ttfc`;
//! `GET /admin/metrics` reads the histogram back. The window is the process lifetime, and the read
//! carries `since` so the window is visible rather than implied.
//!
//! No persistence and no per-client state: the spec blesses in-memory with loss on restart, and
//! the beacon is flood-controlled by the Api rate-limit bucket plus the body caps below. A daily
//! table would add the site's only unauthenticated Postgres write path for an ego metric.
//!
//! The histogram has a fixed set of edges, so 10,000 posts grow nothing (a unit test pins it), and
//! percentiles are the upper edge of the bucket the value landed in: an approximation, named here
//! and in the read's own field names (`p50_ms`, not `median`).

use super::json_ok;
use crate::net;
use crate::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse as _, Response};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// The bucket upper edges, in milliseconds. `TTFC_MAX_MS` is the last edge: a longer value is
/// refused rather than clamped, so a page that reports garbage is visible, not silently averaged.
pub const TTFC_EDGES_MS: [u32; 15] = [
    250, 500, 750, 1000, 1500, 2000, 3000, 4000, 6000, 8000, 10000, 15000, 30000, 60000, 120000,
];

/// The largest accepted value: the last edge.
pub const TTFC_MAX_MS: u32 = TTFC_EDGES_MS[TTFC_EDGES_MS.len() - 1];

/// The beacon body ceiling. The wire type has two small fields; 1000 bytes fits it with room to
/// spare, and `post_guard`'s 300,000 is the outer bound for every POST.
pub const TTFC_MAX_BODY: usize = 1000;

/// The chart engines a beacon can name. One `from_wire`, so an unknown name is refused in one
/// place; `as_str` is the key in the read, so the names exist once as `&'static str`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Engine {
    Vela,
    Lightweight,
    Klinechart,
}

impl Engine {
    pub const ALL: [Engine; 3] = [Engine::Vela, Engine::Lightweight, Engine::Klinechart];

    pub fn from_wire(s: &str) -> Option<Engine> {
        match s {
            "vela" => Some(Engine::Vela),
            "lightweight" => Some(Engine::Lightweight),
            "klinechart" => Some(Engine::Klinechart),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Vela => "vela",
            Engine::Lightweight => "lightweight",
            Engine::Klinechart => "klinechart",
        }
    }
}

/// One engine's bucket counts. Index `i` counts values `<= TTFC_EDGES_MS[i]`.
///
/// `Default` is written by hand, not derived: a derived `Default` gives `counts: vec![]`, which is a
/// histogram with no buckets waiting to be indexed out of bounds. The invariant is that `counts`
/// always has one slot per edge, so `new()` and `default()` are the same value and `or_default()` in
/// `record` is safe.
#[derive(Clone, Debug)]
struct TtfcHist {
    counts: Vec<u32>,
}

impl Default for TtfcHist {
    fn default() -> TtfcHist {
        TtfcHist::new()
    }
}

impl TtfcHist {
    fn new() -> TtfcHist {
        TtfcHist {
            counts: vec![0; TTFC_EDGES_MS.len()],
        }
    }

    /// The bucket index for `ms`, or `None` when it is over [`TTFC_MAX_MS`].
    fn index(ms: u32) -> Option<usize> {
        TTFC_EDGES_MS.iter().position(|e| ms <= *e)
    }

    fn record(&mut self, ms: u32) {
        if let Some(i) = Self::index(ms) {
            self.counts[i] = self.counts[i].saturating_add(1);
        }
    }

    fn total(&self) -> u32 {
        self.counts.iter().fold(0, |a, c| a.saturating_add(*c))
    }

    /// The upper edge of the bucket the `pct` percentile falls in; `None` when empty.
    ///
    /// The approximation: a value is reported as the edge above it, so the read never claims more
    /// precision than the buckets hold.
    fn percentile(&self, pct: u32) -> Option<u32> {
        let total = self.total();
        if total == 0 {
            return None;
        }
        // The rank to reach: at least one observation. `div_ceil` rounds up, so a count that is not
        // a whole rank still demands the next observation rather than falling short of it.
        let rank = (total as u64 * pct as u64).div_ceil(100);
        let rank = rank.max(1) as u32;
        let mut seen = 0u32;
        for (i, c) in self.counts.iter().enumerate() {
            seen = seen.saturating_add(*c);
            if seen >= rank {
                return Some(TTFC_EDGES_MS[i]);
            }
        }
        Some(TTFC_MAX_MS)
    }
}

/// The histogram, one per engine, plus the process start the window is measured from.
///
/// One `std::sync::Mutex`, never held across an `.await` (the feed state's rule).
#[derive(Clone)]
pub struct Ttfc {
    inner: Arc<Mutex<HashMap<Engine, TtfcHist>>>,
    since: i64,
}

/// What `GET /admin/metrics` returns.
#[derive(Clone, Debug, Serialize)]
pub struct TtfcView {
    /// Unix milliseconds of process start: the window this read covers.
    pub since: i64,
    pub engines: BTreeMap<&'static str, TtfcEngineView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TtfcEngineView {
    pub count: u32,
    pub p50_ms: Option<u32>,
    pub p75_ms: Option<u32>,
    pub p95_ms: Option<u32>,
}

/// The beacon body. `deny_unknown_fields`: a page or a probe that adds a field is refused, so the
/// stored shape cannot drift away from the read.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtfcBeacon {
    pub ms: u32,
    pub engine: String,
}

impl Ttfc {
    /// `since_ms` is the process start in unix milliseconds.
    pub fn new(since_ms: i64) -> Ttfc {
        Ttfc {
            inner: Arc::new(Mutex::new(HashMap::new())),
            since: since_ms,
        }
    }

    /// Records one sample; an over-range value is dropped (the handler refuses it first).
    pub fn record(&self, engine: Engine, ms: u32) {
        let mut m = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        m.entry(engine).or_default().record(ms);
    }

    /// The read: every engine that has a sample, in name order.
    pub fn view(&self) -> TtfcView {
        let m = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let mut engines = BTreeMap::new();
        for (e, h) in m.iter() {
            engines.insert(
                e.as_str(),
                TtfcEngineView {
                    count: h.total(),
                    p50_ms: h.percentile(50),
                    p75_ms: h.percentile(75),
                    p95_ms: h.percentile(95),
                },
            );
        }
        TtfcView {
            since: self.since,
            engines,
        }
    }
}

/// `POST /api/metrics/ttfc`: one beacon, 204 with no body.
///
/// The order is the guard's: `post_guard` (Origin, JSON, 300,000 bytes) first, then this route's own
/// caps, then the parse, then the record. The beacon sits outside every gate area by design (it is
/// anonymous, and it guards itself), so it is the one POST that must not skip `post_guard`.
pub async fn ttfc(State(st): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if let Err(resp) = net::post_guard(&st.cfg, &headers, body.len()) {
        return *resp;
    }
    if body.len() > TTFC_MAX_BODY {
        return super::api_error(StatusCode::PAYLOAD_TOO_LARGE, "too large");
    }
    let beacon: TtfcBeacon = match serde_json::from_slice(&body) {
        Ok(b) => b,
        Err(_) => return super::api_error(StatusCode::BAD_REQUEST, "invalid body"),
    };
    if beacon.ms > TTFC_MAX_MS {
        return super::api_error(StatusCode::BAD_REQUEST, "ms out of range");
    }
    let Some(engine) = Engine::from_wire(&beacon.engine) else {
        return super::api_error(StatusCode::BAD_REQUEST, "unknown engine");
    };
    st.ttfc.record(engine, beacon.ms);
    (StatusCode::NO_CONTENT, ()).into_response()
}

/// `GET /admin/metrics`: the histogram read. The gate already answered 404 to everyone who is not
/// a signed-in admin, so this handler has no check of its own.
pub async fn admin_metrics(State(st): State<AppState>) -> Response {
    json_ok(&st.ttfc.view())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engines_round_trip_and_reject_unknown_names() {
        for e in Engine::ALL {
            assert_eq!(Engine::from_wire(e.as_str()), Some(e));
        }
        for bad in ["", "Vela", "vela ", "kline", "pine"] {
            assert_eq!(Engine::from_wire(bad), None, "{bad}");
        }
    }

    #[test]
    fn percentiles_are_the_upper_edge_of_the_bucket() {
        let mut h = TtfcHist::new();
        assert_eq!(h.total(), 0);
        assert_eq!(
            h.percentile(50),
            None,
            "an empty histogram has no percentiles"
        );
        // One sample per bucket is a readable ladder, and the rank is the ceiling of total*pct/100.
        for edge in TTFC_EDGES_MS {
            h.record(edge);
        }
        assert_eq!(h.total(), TTFC_EDGES_MS.len() as u32);
        // ceil(15*50/100) = 8, the 8th bucket, edge 4000.
        assert_eq!(h.percentile(50), Some(4000));
        // ceil(15*75/100) = 12, the 12th bucket, edge 15000.
        assert_eq!(h.percentile(75), Some(15_000));
        // ceil(15*95/100) = 15, the last bucket, edge 120000.
        assert_eq!(h.percentile(95), Some(120_000));

        // A single fast sample: every percentile is that bucket, not zero.
        let mut one = TtfcHist::new();
        one.record(10);
        for pct in [1, 50, 95, 100] {
            assert_eq!(one.percentile(pct), Some(250), "{pct}");
        }
        // Values sit in the bucket at or above them.
        assert_eq!(TtfcHist::index(250), Some(0));
        assert_eq!(TtfcHist::index(251), Some(1));
        assert_eq!(TtfcHist::index(120_000), Some(14));
        assert_eq!(TtfcHist::index(120_001), None, "over the last edge");
    }

    #[test]
    fn ten_thousand_posts_grow_nothing() {
        let t = Ttfc::new(1_700_000_000_000);
        for i in 0..10_000 {
            t.record(Engine::Vela, (i % 2000) as u32);
        }
        let v = t.view();
        assert_eq!(v.engines.len(), 1, "one engine, one histogram");
        assert_eq!(v.engines["vela"].count, 10_000);
        assert_eq!(TTFC_EDGES_MS.len(), 15, "the bucket count never changes");
        // The `since` is the caller's window, passed through unchanged.
        assert_eq!(v.since, 1_700_000_000_000);
    }

    /// The regression this exists for: a derived `Default` gave `counts: vec![]`, so the first
    /// `or_default()` insert panicked on `counts[i]` with "len is 0 but the index is 10". It passed
    /// because nothing recorded into a default-constructed histogram, which is exactly what the
    /// first beacon of a process does.
    #[test]
    fn a_default_histogram_has_a_slot_per_edge() {
        let h = TtfcHist::default();
        assert_eq!(h.counts.len(), TTFC_EDGES_MS.len(), "one slot per edge");
        assert_eq!(h.total(), 0);
        // Recording into a fresh default must not panic, for every engine and every bucket.
        for edge in TTFC_EDGES_MS {
            let mut h = TtfcHist::default();
            h.record(edge);
            assert_eq!(h.total(), 1, "{edge}");
        }
        // And the same through the map path the handler uses.
        let t = Ttfc::new(0);
        t.record(Engine::Vela, 120_000);
        assert_eq!(t.view().engines["vela"].count, 1);
    }

    #[test]
    fn every_engine_keeps_its_own_counts() {
        let t = Ttfc::new(0);
        t.record(Engine::Vela, 500);
        t.record(Engine::Klinechart, 900);
        t.record(Engine::Klinechart, 1200);
        let v = t.view();
        assert_eq!(v.engines["vela"].count, 1);
        assert_eq!(v.engines["klinechart"].count, 2);
        assert!(!v.engines.contains_key("lightweight"), "no sample, no row");
        // Name order, so the JSON body is stable between reads.
        let keys: Vec<&str> = v.engines.keys().copied().collect();
        assert_eq!(keys, vec!["klinechart", "vela"]);
    }
}
