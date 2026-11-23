---
phase: 02-orderflow
plan: 02
status: UNIFIED
applied: 2026-11-23
unified: 2026-11-23
type: Summary
about: "vtradingview"
---

# 02-02 SUMMARY: Binance loading, the trade store, the OrderFlow service

## Result

`OrderFlow::answer(&Ask::parse(kind, query, now))` loads what each computation needs and returns
Python's JSON for all 25 calls of the 02-01 fixtures. The run goes through the real shell: argument
parsing, the locked rows cache, klines paging over HTTP, the history loop through `BarsCache`, and
the footprint's chart-bars call. The trade store gives Python's `trades()` result on all 7 recorded
store scenarios, and its lifecycle (loops, saves, idle stop, restart, reload, day retry, drop) is
tested under the paused clock.

## Tasks

| Task | Status | Qualify |
|---|---|---|
| 1. Endpoints, WallClock, Fapi, weight budget, futures list | DONE | PASS: budget and ban tables under the paused clock, one real-request test against a local fake (429 with Retry-After, the weight header, urllib's error text), futures single flight with TTL and retry |
| 2. Trade store, hour cache, daily archive | DONE | PASS: 7/7 Python store scenarios; segment join order-independent and idempotent; codec round trip and corrupt-file rejection; head and back-fill plans; lifecycle test |
| 3. The OrderFlow service and Ask::parse | DONE_WITH_CONCERNS | PASS: replay 25/0/0; concern resolved: the feed-key unit test first passed its own mutation (deviation 6) |

## Acceptance criteria

| AC | Result | Evidence |
|---|---|---|
| AC-1 budget and futures list | pass | Chart stops at 1,900 and Background at 1,500 within 60 s; a 1790 header stops Background and is cleared; 2190 stops Chart; 429 with Retry-After 7 blocks both priorities, the wait is clamped to 5 s, free at 7 s; 418 without the header blocks 60 s. A real 429 from a local fake bans both priorities and reads `HTTPError: HTTP Error 429: Too Many Requests`. FuturesList: 8 concurrent callers make 1 fetch; refreshed after 6 h; a failed refresh keeps the old set; retried after 5 min, not before. No std lock is held across an await (the lock wraps only `decide` and `note_reply`). |
| AC-2 trade store equals Python | pass | `store scenarios: 7 calls equal Python`: trades bit for bit and in order (15,801 from the XAU day file, 11,493 across the BTC REST gap, none on busy days), covered, pending, floor_ms, day_busy and days_ready after the call. Segments: chunks added in reverse order and then again give the same single segment with every id once. |
| AC-3 lifecycle and caches | pass | Paused clock, in-memory trade source, temp cache dir: the first request starts the loops; within 120 s the window is complete (not pending), every trade is there once and in order, and it starts at t0. The head polls about every 4 s by `fromId` and never asks for the newest page while it is fresh. Complete hours are saved as `.bin` files with no `.part` left. The loops stop after 600 s idle, and a request after the stop starts them again. A new store reloads the saved hours before any REST call. A failed day is not retried (and not pending) within 30 min, and is retried after. Once the store is dropped, no request is made in the next 60 s. |
| AC-4 the service equals Python | pass | `replay: 25 calls, 0 failed, 0 tolerance used`. The fake fapi serves the recorded pages by their exact query, and any unrecorded history call fails the test. Ask::parse unit test: Python's defaults (footprint from now-3h, profile va 68 gives 0.68, a blank value is absent) and error order (rows before va; footprint row_size kept raw). Mutations: a Cached freshness rule FAILED `forming_pages_are_refetched_and_old_pages_cached`; a feed dropped from the key FAILED `the_same_ticker_on_two_feeds_has_its_own_rows`. Both were applied, run, restored and touched (L-009, L-013). |

Regression: the Phase 01 contract against the live Python hub gave `contract: 25 passed, 0 skipped,
0 failed`, with 979 instruments matching exactly. Gates: fmt 0, clippy 0, `cargo test --workspace` 0
(83 passed, 0 failed).

## Deviations (each verified)

1. **`TradeSource` trait in the store.** The store reads `aggTrades` pages and daily archives through
   a two-method trait that `Fapi` implements. The reason is that tokio's paused clock skips ahead
   while a task waits on a socket, which can fire request timeouts before a local fake answers. The
   lifecycle test uses an in-memory source.
2. **`TradeWindows` trait in the service.** `OrderFlow::with_parts` takes the trade windows as a
   trait object (`Trades` in production), so the replay feeds Python's recorded windows rather than
   a live store whose loops would make results depend on timing.
3. **`FuturesList` takes its fetch as a function** (`from_fapi` in production), so the 6 h and 5 min
   rules are tested under the paused clock without sockets.
4. **The lower-bar cache rules live in `calc.rs`** (from 02-01); `RowsCache` wraps them in an LRU of
   256 keys behind one std lock.
5. **Bounded memory (D-009):** day arrays older than today-2 are dropped on the 60 s save, because
   `trades()` never reads them. The rows cache is LRU-bounded. An idle stop keeps the segments in
   memory and saves the hours, as Python does.
6. **The feed-key test was tightened after its mutation passed.** Python's cache always re-reads the
   last cached bar, so a second feed still made a history call with the key shared. The test now
   asserts that the second feed's first request asks for the whole span (the same size as the
   first feed's). The mutation then FAILED (L-015, second hit).
7. **Upstream error classes by prefix:** `HTTP Error` gives HTTPError, `<urlopen error` gives
   URLError, `timed out` gives TimeoutError, anything else RuntimeError (D-009 note; status 502 as
   Python).
8. **Test expectation corrected:** the failed-day check first used yesterday, which is inside the
   30 h REST reach, so the window is pending in Python too. It now uses the day before yesterday.

## Security guidance (/paul-security, optional row, security_audit on)

- **Outbound URLs** are fixed hosts from `Endpoints`. The symbol is always a member of the futures
  set (`real_symbol` returns only listed names), and ids and times are integers, so no request
  input reaches a URL unchecked.
- **Cache paths** are built only from that symbol plus numbers (civil day, `YYYYMMDDHH`). Files are
  written to `.part` and renamed.
- **The daily archive:** only the first entry is read, into memory, and never extracted to disk
  paths (no zip slip). Its size is not capped, as in Python; the host is Binance's public archive.
  The `zip` crate is built with deflate only.
- No secrets are involved; all data is public market data, read-only.

## Skill audit

- `/architect`: plan stage. `pstack_stage: plan; APPLY ran in-session`.
- `/paul-security`: guidance recorded above (optional row).

## Reflect and RSI

Deferred to phase close (reflect: pstack; 02-02 is not the last plan of Phase 02).

## Close-out

- rules: +0, decisions: +0
- learnings: +1 added (L-016), 2 bumped (L-015, L-001), 0 promoted, 0 removed
- debt: +0 added, 0 discharged; DEBT-007 tightened to include the 02-02 files

**Open risk: UNIFIED is not LANDED.** The Phase 02 files are uncommitted until the owner commits
(DEBT-007).

## Files

Created: `crates/hub-core/src/orderflow/trades.rs`, `crates/hub-core/tests/{orderflow_store,orderflow_replay}.rs`,
`crates/hub-core/tests/common/mod.rs`. Modified: `crates/hub-core/src/orderflow/{mod,binance}.rs`,
`crates/hub-core/src/{clock,config}.rs`, `crates/hub-core/src/sources/mod.rs`,
`crates/hub-core/tests/ws_e2e.rs`, `crates/hub-core/Cargo.toml` (`zip` regular dependency).

## Next-plan readiness (02-03)

Ready. 02-03 wires `OrderFlow::new(bars, sources.endpoints(), WallClock::System, cache dir)` into
`AppState`, mounts the three routes on `Ask::parse` and `OrderFlow::answer`, and logs a failure
through `Kind`'s display and `OfError::msg()`.

## Erratum (02-03)

The "same key order" claim above was not checked when written: `serde_json` sorted the keys of
parsed objects (no `preserve_order`), so the comparator compared sorted key sets. 02-03 enabled
`preserve_order` for test builds; the suites still pass with real key order, and moving one field
fails them. See 02-03-SUMMARY deviation 1 and L-017.
