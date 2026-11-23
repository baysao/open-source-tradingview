//! AC-1 of 01-02: recorded upstream frames replayed through the Rust parsers and `BarBuilder` give
//! exactly the bars Python's stream logic gives for the same frames.
//!
//! Fixtures were recorded live (Binance BTCUSDT aggTrade, Bybit BTCUSDT publicTrade, Deriv R_100
//! ticks) with the expected bars computed by a copy of the rules in `hub/server.py`.

use hub_core::market::candles::BarBuilder;
use hub_core::market::Ticker;
use hub_core::sources::{binance, bybit, deriv, parse_frame, Parsed, UpstreamKey};
use hub_types::market::{Bar, Source, Tf};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Expected {
    seed: Option<Bar>,
    bars: Vec<Option<Bar>>,
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn replay(
    name: &str,
    source: Source,
    symbol: &str,
    late: hub_core::market::candles::LatePolicy,
) -> usize {
    let frames = std::fs::read_to_string(fixtures().join(format!("{name}.jsonl"))).unwrap();
    let expected: Expected = serde_json::from_str(
        &std::fs::read_to_string(fixtures().join(format!("{name}.expected.json"))).unwrap(),
    )
    .unwrap();
    let key = UpstreamKey {
        source,
        symbol: Ticker::parse(source, symbol).unwrap(),
    };
    let mut builder = BarBuilder::seed(Tf::M1, expected.seed);
    let lines: Vec<&str> = frames.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        expected.bars.len(),
        "{name}: one expected bar per frame"
    );
    let mut batches = 0;
    for (i, (line, want)) in lines.iter().zip(&expected.bars).enumerate() {
        if let Parsed::Batch(b) = parse_frame(&key, line) {
            batches += 1;
            for t in &b.ticks {
                builder.apply(t, late);
            }
        }
        assert_eq!(builder.current(), *want, "{name}: frame {i} {line}");
    }
    batches
}

#[test]
fn binance_aggtrade_matches_python() {
    assert!(
        replay(
            "binance_aggtrade",
            Source::Binance,
            "BTCUSD",
            binance::RULES.late
        ) > 0
    );
}

#[test]
fn bybit_publictrade_matches_python() {
    assert!(
        replay(
            "bybit_publictrade",
            Source::Bybit,
            "BYBIT_BTCUSDT",
            bybit::RULES.late
        ) > 0
    );
}

#[test]
fn deriv_ticks_match_python() {
    assert!(replay("deriv_ticks", Source::Deriv, "R_100", deriv::RULES.late) > 0);
}
