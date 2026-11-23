//! AC-6 of 01-02 (Q-ARCH-5): the single `HubState` lock under load.
//!
//! 400 upstreams with 4 viewers each replay the recorded Binance BTCUSDT frames at 10x real rate,
//! concurrently, through the same path the session loop uses (lock, ingest, unlock, serialize once,
//! broadcast). Measures the time from a frame's scheduled arrival to the end of its fan-out, and
//! checks that every viewer of an update receives the same serialized buffer (one serialization per
//! admitted update, whatever the viewer count).
//!
//! Ignored by default (it runs for about 3 s of wall time and prints numbers for the SUMMARY):
//! `cargo test -p hub-core --release --test replay_load -- --ignored --nocapture`

use hub_core::feeds::hub::frame;
use hub_core::feeds::state::{HubState, Limits};
use hub_core::market::Ticker;
use hub_core::sources::{binance, parse_frame, Parsed, UpstreamKey};
use hub_types::market::{Bar, Source, Tf};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::Instant;

const UPSTREAMS: usize = 400;
const VIEWERS: usize = 4;
const SPEEDUP: f64 = 10.0;

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore]
async fn four_hundred_upstreams_four_viewers_at_ten_x() {
    let raw = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/binance_aggtrade.jsonl"),
    )
    .unwrap();
    let probe = UpstreamKey {
        source: Source::Binance,
        symbol: Ticker::parse(Source::Binance, "BTCUSD").unwrap(),
    };
    let batches: Vec<_> = raw
        .lines()
        .filter_map(|l| match parse_frame(&probe, l) {
            Parsed::Batch(b) => Some(b),
            _ => None,
        })
        .collect();
    let t_first = batches[0].src_ms;
    let state = Arc::new(Mutex::new(HubState::new(Limits {
        max_upstreams: UPSTREAMS,
        ..Limits::default()
    })));
    let seed = Bar {
        time: (t_first / 1000) - (t_first / 1000) % 60 - 60,
        open: 1.0,
        high: 1.0,
        low: 1.0,
        close: 1.0,
        volume: 0.0,
    };

    let mut keys = Vec::new();
    let mut viewers = Vec::new();
    for i in 0..UPSTREAMS {
        let key = UpstreamKey {
            source: Source::Binance,
            symbol: Ticker::parse(Source::Binance, &format!("S{i}USDT")).unwrap(),
        };
        let mut epoch = 0;
        for _ in 0..VIEWERS {
            let j = state
                .lock()
                .unwrap()
                .join(&key, binance::RULES, Tf::M1, Some(seed), Instant::now())
                .unwrap();
            epoch = j.epoch;
            viewers.push(j.rx);
        }
        keys.push((key, epoch));
    }

    let received = Arc::new(AtomicU64::new(0));
    let shared_mismatch = Arc::new(AtomicU64::new(0));
    // viewers: count frames; group by upstream to check buffer sharing
    let mut viewer_tasks = Vec::new();
    for chunk in viewers.chunks_mut(VIEWERS) {
        let mut rxs: Vec<_> = chunk.iter_mut().map(|r| r.resubscribe()).collect();
        let received = received.clone();
        let mismatch = shared_mismatch.clone();
        viewer_tasks.push(tokio::spawn(async move {
            loop {
                let mut ptrs = Vec::with_capacity(VIEWERS);
                for rx in rxs.iter_mut() {
                    match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                        Ok(Ok(f)) => {
                            received.fetch_add(1, Ordering::Relaxed);
                            ptrs.push(f.as_str().as_ptr() as usize);
                        }
                        _ => return,
                    }
                }
                if ptrs.windows(2).any(|w| w[0] != w[1]) {
                    mismatch.fetch_add(1, Ordering::Relaxed);
                }
            }
        }));
    }
    drop(viewers);

    let start = Instant::now();
    let serialized = Arc::new(AtomicU64::new(0));
    let mut producers = Vec::new();
    for (key, epoch) in keys {
        let state = state.clone();
        let batches = batches.clone();
        let serialized = serialized.clone();
        producers.push(tokio::spawn(async move {
            let mut lat = Vec::with_capacity(batches.len());
            let mut work = Vec::with_capacity(batches.len());
            for b in &batches {
                let due =
                    start + Duration::from_millis(((b.src_ms - t_first) as f64 / SPEEDUP) as u64);
                tokio::time::sleep_until(due).await;
                let woke = Instant::now();
                let now_ms = (b.src_ms + 20) as f64;
                let emits = state.lock().unwrap().ingest(
                    &key,
                    epoch,
                    b,
                    Instant::now(),
                    now_ms,
                    now_ms as i64,
                );
                for e in emits {
                    let f = frame(&e.msg);
                    serialized.fetch_add(1, Ordering::Relaxed);
                    let _ = e.tx.send(f);
                }
                lat.push(Instant::now().duration_since(due));
                work.push(Instant::now().duration_since(woke));
            }
            (lat, work)
        }));
    }
    let mut all = Vec::new();
    let mut work_all = Vec::new();
    for p in producers {
        let (l, w) = p.await.unwrap();
        all.extend(l);
        work_all.extend(w);
    }
    work_all.sort();
    let wp = |q: f64| work_all[((work_all.len() as f64 * q) as usize).min(work_all.len() - 1)];
    println!(
        "replay_load: processing after wake-up (lock, ingest, serialize, broadcast): p50 {:?} p99 {:?} max {:?}",
        wp(0.50),
        wp(0.99),
        work_all[work_all.len() - 1]
    );
    for v in viewer_tasks {
        let _ = v.await;
    }
    all.sort();
    let pct = |q: f64| all[((all.len() as f64 * q) as usize).min(all.len() - 1)];
    let ser = serialized.load(Ordering::Relaxed);
    let rec = received.load(Ordering::Relaxed);
    println!(
        "replay_load: {} batches over {UPSTREAMS} upstreams x {VIEWERS} viewers in {:?}; latency p50 {:?} p99 {:?} max {:?}; serialized {ser}, delivered {rec} ({:.2} deliveries per serialization), shared-buffer mismatches {}",
        all.len(),
        start.elapsed(),
        pct(0.50),
        pct(0.99),
        all[all.len() - 1],
        rec as f64 / ser.max(1) as f64,
        shared_mismatch.load(Ordering::Relaxed)
    );
    assert!(
        pct(0.99) <= Duration::from_millis(50),
        "p99 processing latency over 50 ms"
    );
    assert_eq!(rec, ser * VIEWERS as u64, "every viewer got every frame");
    assert_eq!(
        shared_mismatch.load(Ordering::Relaxed),
        0,
        "viewers of one update share one buffer"
    );
}
