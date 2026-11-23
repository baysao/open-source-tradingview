---
phase: 02-orderflow
type: Architecture
about: "vtradingview"
date: 2026-11-23
---

# Phase 02 architecture: order flow in Rust

Synthesized by `/architect` (arena, three runners plus one cross-judge, all on the parent model per
`~/.agents/pstack-models.md`). Candidates and the judge report:
`scratchpad/architect-p02/candidate-{1,2,3}/design.md` (session scratchpad, not committed).

## Problem

Port `hub/orderflow_api.py`, `orderflow_trades.py`, `orderflow.py` and the three routes
(`hub/server.py:921-968`) so the Rust output equals Python's: rounded fields exactly, the unrounded
floats (`lo`, `step`, `poc`, `vah`, `val`, `row_size`) within 1e-9 relative. Spec facts:
`.paul/design/specs/vtradingview.md` "Phase 2 facts". Four things make the shape non-obvious, all
measured on this machine (Python 3.12.10, numpy 2.5.0, pandas 3.0.6):

1. **Float results depend on order.** Python 3.12's builtin `sum()` is Neumaier-compensated (a plain
   fold differed on 1,997 of 3,000 probe lists); the footprint fill adds `buy / n` per cell in the
   lower-bar dict's insertion order (`orderflow_api.py:128-140, 389, 417, 426`), which is not time
   order once a request reaches before the cached start.
2. **Number kinds show in the JSON.** `nice_step` returns an int for 1, 2, 5, 10 x 10^k with k >= 0
   (`row_size: 5`), and an empty footprint row is int `0` (`orderflow_api.py:358-362, 432`).
3. **The trade store depends on threads and the clock.** `trades()` reads `time.time()` and
   `datetime.now()` and starts loaders (`orderflow_trades.py:295-319`); a seeded store with frozen
   clocks and loaders disabled is deterministic.
4. **Gold maps differently.** The hub's `Ticker::parse(Binance, "XAUUSD")` maps gold to `PAXGUSDT`
   (`sources/binance.rs:41`); order flow maps it to `XAUUSDT`. The raw chart ticker must reach
   `real_symbol`.

Constraints: no parquet/arrow, `zip` (deflate) the only new crate; frontend, Python modules, route
parameters and keys unchanged; goldens without network and with a frozen clock; `std::sync::Mutex`
never across `.await`; tokio `Instant` (L-001); init state and start order from the oracle (L-011);
tests write no files (L-008).

## Usage (caller's view)

```rust
// routes/orderflow.rs (02-03): the whole handler, mounted on the three paths
pub async fn serve(kind: Kind, st: AppState, q: HashMap<String, String>) -> Response {
    let of = &st.orderflow;
    let res = match Ask::parse(kind, &q, of.now_s()) {
        Ok(ask) => of.answer(&ask).await,
        Err(e) => Err(e),
    };
    match res {
        Ok(reply) => json_ok(&reply),
        Err(e) => fail(kind, &q, e),
    }
}
// fail: tracing::warn!(target: "server", every = 30, "orderflow {kind} {sym} {tf}: {msg}")
//       then api_error(502, "<Class>: <msg>")

// tests/orderflow_golden.rs (02-01): pure, no fake, no clock
let fx = Fixture::load("profile_xau_deriv_m5_sessions");
let w = calc::profile_window(&fx.ask_profile(), fx.from_s, fx.to_s)?;      // windows checked too
assert_eq!(w.lower, fx.recorded_rows_args("real", "M1"));
let lower = calc::real_rows(&fx.klines_for(w.lower));
let chart = calc::estimated_rows(&fx.history_chunks(w.chart), w.chart);
let got = calc::profile(&w, &fx.opts(), &fx.resolved(), &lower, Some(&chart))?;
assert_same(&fx.expected, &serde_json::to_value(&got)?);                   // bit-exact, number kinds

// tests/orderflow_replay.rs (02-02): the shell against fakes, same fixtures
let of = OrderFlow::new(BarsCache::new(Arc::new(ReplayHistory::strict(&fx))),
                        &Endpoints { binance_fapi: fake.url(), ..Endpoints::default() },
                        WallClock::frozen(fx.now_ms), None);
assert_same(&fx.expected, &serde_json::to_value(of.answer(&fx.ask()).await?)?);
```

Public surface: `OrderFlow::{new, now_s, answer}`, `Ask::parse`, `Ask`, `Kind`, `Reply`, `OfError`,
`WallClock`, the pure `calc` functions the goldens call, the reply types in `hub-types`.

## Shape

### Data structures

```text
Chart   { raw: String, feed: Feed }                 raw ticker feeds real_symbol; feed parses it for history
Feed    = Live(Source) | Off(Mt5 | Arm)             {"mt5","deriv","arm","bybit"}.get(feed, binance)
Ask     = Delta(Common) | Profile(Common, ProfileOpts) | Footprint(Common, FootprintOpts)
Common  { chart: Chart, tf: TfArg, from_s, to_s, pick: Pick(Auto|Real|Estimated) }
TfArg   raw string, resolved where Python first indexes TF_SEC (error precedence kept)
Resolved{ src: Real|Estimated, real: Option<FuturesSymbol> }  FuturesSymbol only from resolve()

Lower     { buy, sell, sign, high, low, close }     Python's 6-tuple
LowerRows = IndexMap<i64, Lower>                    DICT SEMANTICS: insert keeps an existing key's place;
                                                    never sorted in place; sort explicitly where Python sorts
Kline     { open_ms, high, low, close, volume, taker_buy }
TradeCols { t_ms, price, qty, side }                store concatenation order kept
TradeWindow { trades: TradeCols, covered: Vec<(i64,i64)>, pending: bool }

Num = Int(i64) | Float(f64)                         row_size kind decided inside nice_step
Vol = Gap | V(f64)                                  Gap serializes as int 0
Chunk { ids, t, p, q, s }  Arc, immutable            Segments: Vec<Seg{chunks, t0, t1}>, sorted, ids disjoint
```

### Module map (amends spec item 16: five files plus the route, grouped by knowledge)

```text
crates/hub-types/src/orderflow.rs   02-01  reply structs (deny_unknown_fields), Num, Vol, FlowSource
crates/hub-core/src/orderflow/
  mod.rs      02-01 OfError, PyClass, Ask/Kind/Common/TfArg/Chart/Feed types; 02-02 OrderFlow, Reply,
              RowsCache, history loop, Ask::parse
  calc.rs     02-01 every Python number: py_sum (Neumaier), py_round, py_mod, py_max/min, py_sort,
              py_int, py_float, nice_step; real_symbol/resolve, windows, real_rows, estimated_rows,
              need_volume, shift, delta, session_of, histogram, value_area, levels, profile,
              auto_row_size, footprint
  binance.rs  02-01 parse_klines, parse_exchange_info, parse_agg_trades (wire DTOs private);
              02-02 Fapi (reqwest), WeightState + Weight, FuturesList
  trades.rs   02-02 Segments (pure join), union/covers, window assembly (pure, returns effects),
              D-008 column codec, daily zip + CSV, SymbolStore loops, Trades registry
crates/hub-core/src/routes/orderflow.rs  02-03 serve(kind, state, query)
```

A request traces through three files: `routes/orderflow.rs` -> `orderflow/mod.rs` -> `calc.rs`.

### Load-bearing decisions

1. **The pure core takes loaded inputs** (`LowerRows`, history chunks, `TradeWindow`), and the
   windows (alignment, `MAX_ROWS` clamp, whole-session snap) are pure too: goldens need no fake, and
   the shell decides only what to load (boundary-discipline).
2. **Python numerics are named functions in `calc.rs`**, each table-tested against values Python
   prints; `round` is `format!("{:.n$}")` + `parse` (probe: 0 mismatches in 210,008 values with
   9,998 exact ties). A clippy `disallowed-methods` rule for `f64::max`, `f64::min` and
   `Iterator::sum` inside `orderflow/` makes a bypass a build error (encode-lessons-in-structure).
3. **Insertion order is kept where Python keeps it** (`LowerRows` is an `IndexMap`; `RowsCache`
   merges with `insert`, `window` filters without sorting); delta, profile and shift sort exactly
   where Python sorts. The golden `warm_cache_sequence` case proves it.
4. **Number kinds are types** (`Num`, `Vol`); the comparator checks bits and int-vs-float, and prints
   `tolerance used at <path>` whenever the 1e-9 allowance was needed (expected: never).
5. **History through `BarsCache`**, `Freshness::Fresh` when the page reaches the forming bar, else
   `Cached`; parsed from `Chart.raw` with `Ticker::parse(feed, raw)`.
6. **`feed=mt5|arm` is `Feed::Off`** (D-009): the request still runs, so a real-source delta answers
   200 as in Python; anything that calls history answers 502 `RuntimeError: <feed> history is not
   available in this server (D-001)`.
7. **One owner per shared thing.** Weight budget: `Fapi` (std Mutex around a pure `decide`, sleep
   outside). Futures list: `FuturesList` (tokio Mutex, single-flight; empty until the first request,
   as Python). Rows: `RowsCache` (std Mutex; key `(src, raw ticker, tf, feed)`: the feed is added,
   which cannot change a single-feed answer). Trade stores: `Trades` registry; loops hold `Weak`.
8. **Two loops may write `Segments` because `add` is a join** (union of id ranges, min/max
   coverage): idempotent and order-independent, so one short std Mutex is enough. The head poll finds
   its segment by id after the await, never by a reference held across it.
9. **Idle stop and restart are decided under one lock**; an idle stop saves the complete hours and
   keeps memory (as Python); shutdown is ownership (`Weak` upgrade fails after `AppState` drops,
   `AbortOnDrop` for the head loop); hour and day files are `.part` + rename, so a repeated save is
   identical.

## Synthesis decision

- **Base: candidate 1** (cross-judge 28/30, my reading agreed): smallest surface (two calls for a
  route), a pure `calc` the goldens call with no fake, the existing injection points
  (`Endpoints` fakes, `HistoryFetch` behind `BarsCache`, `WallClock`), the cleanest plan split.
- **Grafted from candidate 3:** `IndexMap` lower rows with dict semantics and the
  `warm_cache_sequence` case (its fact check held against the Python source); `Num`/`Vol` number
  kinds and the bit-exact `assert_same`; the measured parity ledger (round, Neumaier sum, bincount
  equals a sequential loop, pandas CSV parse equals `float()` on 300,000 prices); the deferred
  `TfArg` that keeps Python's error precedence; the clippy guard; Python store scenarios
  (`TradeStore` seeded through `_add` and `_day_job`, `thread` set, clocks frozen) as 02-02 goldens.
- **Grafted from candidate 2:** goldens assert the computed windows against the recorded `_rows` and
  `trades()` arguments; the raw-ticker vs parsed-ticker split in a type (`Chart { raw, feed }`).
- **Rejected:** candidate 1's sorted cache and `0.0` gap rows (parity loss); candidate 2's tape
  actor (more machinery than a join-safe mutex store); candidate 3's five-method `Feeds` trait as the
  primary seam (duplicates `Endpoints`/`HistoryFetch`), its idle-drops-memory and abort-without-save
  (keep Python's behaviour), and keeping the feed out of the rows key.

## Tradeoffs accepted

- We accept a dense `calc.rs` (near 800 lines) in exchange for every float-order rule in one file.
- We accept `IndexMap` and explicit sorts in exchange for footprint fill sums in Python's order.
- We accept porting Python quirks (a hole inside the cached span is never refetched; footprint key
  `bt*1e8 + row` spill) verbatim before the parity gate; fixing them later is a decision row each.
- We accept `format!` + `parse` per rounded value in exchange for proven rounding.
- We accept a feed in the rows key (Python omits it) and a bounded rows cache (LRU) in exchange for
  correctness across feeds and bounded memory; neither changes a 200 body.
- We accept fixtures that depend on one network run (pass 1) in exchange for real price grids,
  ties, flat bars and paging; pass 2 and `check` never touch the network.

## Alternatives considered

- **The spec's eleven files:** the same float rules would cross six file boundaries (leakage).
- **Loader traits injected into the pure functions:** async-shaped pure code and a mock per trait.
- **An actor per trade store:** removes a mutex the join already makes safe, adds message plumbing.
- **Goldens only at HTTP level:** 02-01 would wait for 02-02, and a mismatch would not say whether
  the loader or the arithmetic is wrong (kept as the 02-02 replay layer).

## Open questions and risks

- `f64::log10` and `10^k` parity is argued for the MSVC target; a Linux CI run re-proves it through
  the `py_numeric` table and could fail at an exact power of ten (fallback: a lookup of powers).
- Python writes `NaN` (invalid JSON) for `va=nan`; Rust answers 502 `ValueError` (D-009 note).
- Fixture size budget 8 MB; footprint windows 30 to 60 minutes.
- Pass 1 needs the network once (about 2,000 Binance weight, one BTC daily file around 30 MB).

## Plan boundaries

| Plan | Files | Proves |
|---|---|---|
| 02-01 | `hub-types/src/orderflow.rs`, `orderflow/{mod,calc,binance}.rs` (pure half), `lib.rs` (mod decl), `clippy.toml`, `scripts/capture_orderflow_fixtures.py`, `tests/fixtures/orderflow/**`, `tests/orderflow_golden.rs` | numerics tables; every case's windows and JSON equal Python bit for bit |
| 02-02 | `binance.rs` (Fapi, Weight, FuturesList), `trades.rs`, `mod.rs` (OrderFlow, RowsCache, Ask::parse), `clock.rs` (WallClock), `config.rs` (`HUB_CACHE_DIR`), `sources/mod.rs` (`binance_fapi`, `binance_vision`), `Cargo.toml` (`zip`), `tests/orderflow_store.rs`, `tests/orderflow_replay.rs` | weight and futures rules under paused time; store scenarios equal Python's `trades()`; every 02-01 case through the real shell against fakes |
| 02-03 | `routes/orderflow.rs`, `routes/mod.rs`, `lib.rs` (`AppState.orderflow`, routes), `tests/contract/requests.toml`, `tests/contract.rs` | 502 texts and defaults; contract against live Python (shape, plus values on closed past windows); browser check of the order-flow chart types (R-6) |

## Next implementation step

`calc.rs`'s numerics block (`py_sum`, `py_round`, `py_mod`, `py_max`/`py_min`, `py_sort`, `py_int`,
`py_float`, `nice_step`) with tables whose expected values Python 3.12.10 prints.
