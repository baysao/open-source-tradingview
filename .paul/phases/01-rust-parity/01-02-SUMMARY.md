---
phase: 01-rust-parity
plan: 02
status: UNIFIED
applied: 2026-11-23
unified: 2026-11-23
type: Summary
about: "vtradingview"
---

# 01-02 SUMMARY: FeedHub, live feeds, /ws, /api/health

## Result

All three tasks done; AC-1 to AC-6 met. Live `/ws` speaks the Python protocol unchanged: over 20 s
of real Binance BTCUSDT data, Python and Rust sent the same message types with the same field sets.
One upstream serves every viewer of a symbol; the lock path costs microseconds under 400 upstreams.

## Tasks

| Task | Status | Evidence |
|---|---|---|
| 1. Pure core: bar builder, throttle, health, backoff, clock, frame parsers | done | 3 fixture replays match Python bar for bar (80 Binance, 114 Bybit, 13 Deriv frames); unit tests for roll-over, late-trade rules, D1, throttle, backoff, SNTP decoding, health percentiles |
| 2. HubState, FeedHub, upstream session loop, client IP | done | state tests with explicit Instants (one upstream for BTCUSD and BTCUSDT across 100 viewers; grace; rejoin; stale epoch; eviction; per-IP and loopback exemption; closed-bar publish) |
| 3. /ws and /api/health, end-to-end and load tests, contract cases | done | `ws_e2e` passes against fakes; `replay_load` numbers below; contract `/ws` and `/api/health` match Python |

## Acceptance criteria

| AC | Result | Measured |
|---|---|---|
| AC-1 bar rules per source | pass | `fixture_replay`: every frame's bar equals Python's (exact f64); late Binance or Deriv trade merges, late Bybit trade is ignored (unit tests); roll-over emits the closed bar then the new bar (`ingest_throttles_and_publishes_closed_bar_on_roll`, D-004) |
| AC-2 one upstream per (source, symbol) under races | pass | 100 joins over BTCUSD M1 and BTCUSDT M5 start one upstream; rejoin at 30 s reuses it and re-seeds (`rejoin_within_grace_reuses_upstream_and_reseeds_series`); removed only after 60 s idle; stale-epoch leave and ingest are no-ops |
| AC-3 limits | pass | 3rd symbol at cap 2 with none idle: `Capacity` (`kind: "capacity"` frame); with an idle one: longest idle evicted; 25th socket per IP: refused (HTTP 429 before upgrade); loopback not counted while auth is off |
| AC-4 /ws end to end against fakes | pass | Bybit fake: `history` with `bars, source, symbol, tf`; `bar` with `bar, sent, trade_ms, lag_ms`; resubscribe M5 replaces M1; fake drop gives `status "Bybit reconnecting: ..."` then bars again; exactly 2 upstream connections for M1 + M5 + one reconnect; `{"tf":"M2"}` gives `error "Unknown timeframe M2"` and the socket stays open; non-JSON closes the socket |
| AC-5 contract and health | pass | Python and Rust over 20 s, `binance BTCUSDT M1`: `{bar: [bar, lag_ms, sent, trade_ms, type], history: [bars, source, symbol, tf, type]}` on both; `/api/health` top keys equal, entry keys `age_ms, lag_median_ms, lag_p95_ms, ticks` on both |
| AC-6 fan-out cost | pass | release build, 400 upstreams x 4 viewers, 32,000 batches at 10x: processing after wake-up p50 1.9 us, p99 49.7 us, max 0.73 ms; end to end from scheduled arrival p99 19.0 ms (under the 50 ms bound; dominated by the Windows timer tick of about 15.6 ms, not the lock); 4.00 deliveries per serialization; 0 shared-buffer mismatches |

## Decisions and defaults applied

D-002 (protocol, 24 sockets per IP, upstream per symbol), D-004 (closed bar on roll-over). Written
defaults from the plan, all as written: 429 before upgrade; loopback exempt while auth is off;
evict the longest idle at the cap; the link notice replayed to a viewer who joins during an outage;
every provider reconnects with jittered backoff (Binance no longer ends silently); an empty history
no longer ends the stream; `/api/health` lists live upstreams and keys Binance by the mapped symbol.

New: **D-006** (logging convention, below).

## Deviations (each verified)

1. **Exact float parsing.** The fixture replay first failed on the last binary digit of a volume
   sum (`0.00045000000000000004` against `0.00045`). Python's own sum is the longer value; the
   expected file was misread by serde_json's default fast float parser. The workspace now enables
   serde_json `float_roundtrip`, which also makes upstream prices parse exactly. Re-run: 3/3 pass.
2. **Fake seed per interval.** `ws_e2e` first timed out waiting for M5 bars: the fake kline
   endpoint returned a 1-minute-aligned seed for every interval, so Bybit's drop-late rule
   (correctly) ignored every trade for M5. The fake now aligns the seed to the requested interval.
   The hub was right; the test fixture was wrong.
3. **`/ws` and `/api/health` contract cases** are a dedicated test (`ws_and_health_match_python`),
   not rows in `requests.toml`: `/ws` needs a websocket client and `/api/health` has map-shaped keys
   that differ by design (Binance mapped symbol). Same oracle, same skip rule.
4. **Tracing convention (D-006).** Tracing targets must be static strings, so feed events use target
   `feed` with a `src` field (`deriv`, `bybit`, `binance`) instead of a target per provider; 01-03's
   `RingLayer` maps `feed` + `src` to the log source `feed.<src>`. Propagated into the 01-03 plan.
5. **`client_ip` in 01-02** as the architecture planned (the per-IP socket limit needs it);
   `ClientIp` carries `peer_loopback` for the local-mode exemption.

## Skill audit

- `pstack_stage: plan; APPLY ran in-session`; `/architect` loaded at plan time.
- `/paul-security` (optional): not run; covered by the `client_ip` table (header trusted only from
  loopback) and the AC-3 limit tests. Unverifiable (warn-only).

## Reflect and RSI

Deferred to phase close (reflect: pstack; 01-02 is not the last plan of Phase 01).

## Close-out

- rules: +0, decisions: +1 (D-006)
- learnings: +3 added (L-004 to L-006), 0 bumped, 0 promoted, 0 removed
- debt: 0 added, 0 discharged; DEBT-001 check tightened to include this SUMMARY

### Deferred Items (generated from `debt.py --run-checks`)

| Id | Status | Title | Trigger |
|---|---|---|---|
| DEBT-001 | waiting | Commit the Phase 01 work on arch/axum-tauri | event:first-commit |

**Open risk: UNIFIED is not LANDED.** All 01-01 and 01-02 files are uncommitted until the owner
commits.

## Files

Created: `crates/hub-types/src/{ws,health}.rs`; `crates/hub-core/src/{clock,net}.rs`,
`src/market/{candles,feed_health,backoff}.rs`, `src/feeds/{mod,state,hub,upstream}.rs`,
`src/routes/{ws,health}.rs`; `tests/{fixture_replay,ws_e2e,replay_load}.rs`;
`tests/fixtures/{binance_aggtrade,bybit_publictrade,deriv_ticks}.{jsonl,expected.json}`.
Modified: `Cargo.toml` (serde_json `float_roundtrip`), `crates/hub-types/src/lib.rs`,
`crates/hub-core/src/{lib.rs, market/mod.rs, routes/mod.rs, sources/{mod,binance,bybit,deriv}.rs}`,
`tests/contract.rs`.

## Next-plan readiness (01-03)

Ready. 01-03 reads `FeedHub::view` / `FeedView` (per-source `Totals`: updates, ticks, reconnects,
errors; per-upstream health, lags, series viewers, idle, last data), `Catalog::status`,
`BarsCache::stats`, and the `tracing` convention in D-006.
