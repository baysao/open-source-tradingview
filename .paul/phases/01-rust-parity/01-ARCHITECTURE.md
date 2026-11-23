# Phase 01 architecture (synthesized from /architect arena, base candidate B: pure core, thin shell)

> **Reading rule.** Where the "Synthesis decision" section below (grafts and fixes) differs from the type
> sketch further down, the synthesis wins; APPLY implements the synthesis. Decisions D-001 to D-004 in
> `.paul/DECISIONS.md` win over both.

## Problem

Replace the Python hub's HTTP server (`hub/server.py` `LabHandler`, port 8770) and websocket server (port 8765)
for the three free feeds with one Rust process on one port. The browser contract is fixed. The JSON shapes of
`/api/status`, `/api/symbols`, `/api/instruments`, `/api/bars`, `/api/health` and `/api/logs` stay the same, and
so does the `/ws` protocol (one untyped `{source, symbol, tf}` per socket, a new message replaces the stream,
replies `history`, `bar`, `status`, `error`, plus an optional `kind: "capacity"` on `error`, D-002). The
browser may change only `hub/lab.js wsUrl()` and the `hub/hub.html` arm branch. The shape is not obvious for
three reasons, all measured in the code:

1. The Python server is per viewer. Each socket opens its own upstream, its own history fetch, its own bar
   builder and its own 50 ms throttle (`server.py:329-367`, `453-512`, `651-684`). D-002 asks for one upstream
   per `(source, symbol)` with one builder per timeframe, so every per-viewer concept has to be cut in two: the
   part that belongs to the upstream (ticks, health, reconnects) and the part that belongs to the viewer
   (history, socket limit).
2. Feed logging counts at the browser socket today (`feedlog.CountingWS` wraps each viewer). Under shared
   upstreams that multiplies every number by the viewer count. It has to count at the upstream or at the
   fan-out.
3. The work is cut into three plans that must each compile and test alone. 01-01 serves `/api/bars` and
   `/api/symbols`, so it needs the per-provider REST and RPC code. 01-02 needs the live half of the same
   providers. 01-03 needs the log ring, and 01-02 must not depend on it.

Constraints honoured: D-001 (local mode only, bind 127.0.0.1, no MT5 or arm, no `/api/restart`), D-002
(protocol and limits), one router (`hub_core::app`), a thin binary, no database, no auth code, and Phase 3
adding `db` to the state additively. The quirks that still hold today are ported on purpose: Binance merges a
late trade into the current bar and Bybit drops it (`server.py:356-358` against `473-476`); Deriv has no
throttle and a volume of 0; `/api/bars` falls back to Binance for an unknown source; an unknown tf is a 502
`{"error": "'M2'"}`; a JSON parse error closes the socket.

## Usage (caller's view)

The quickstart a contributor reads, `crates/hub-core/README` style:

```text
hub-core is the whole server as a library. There is one entry point:

    let cfg   = Config::from_env()?;                 // the only reader of env vars
    let state = AppState::build(cfg).await?;         // no network I/O; safe in tests
    let _bg   = state.spawn_background();            // symbol refresh, sweeper, feedlog, SNTP
    hub_core::serve(listener, state).await?;          // app(state) with ConnectInfo, nothing else

Every rule that decides what a browser sees is a pure function or a plain struct with a
synchronous method that takes "now" as an argument:

    BarBuilder::apply(&mut self, &Tick) -> Applied            candles.rs
    Throttle::admit(&mut self, Instant) -> bool               candles.rs
    HubState::join / leave / ingest / sweep                   feeds/state.rs
    static_files::resolve(&str) -> Option<StaticTarget>       static_files.rs
    BarsQuery::from_params(&Params) -> Result<BarsQuery, _>   market/bars_query.rs
    instruments::build(&[Symbol]) -> InstrumentsResponse      market/instruments.rs
    binance::parse_frame / bybit::parse_frame / deriv::parse_frame (&str) -> Parsed
    LogRing::push / snapshot,  FeedLog::step                  logbus.rs, feedlog.rs

Test them with recorded frames in tests/fixtures/*.jsonl; nothing in that list opens a socket,
reads a clock or awaits.
```

Call site 1, the `/ws` route (`routes/ws.rs`), the only consumer of `FeedHub`'s subscribe API:

```rust
async fn upgrade(State(st): State<AppState>, ClientIp(ip): ClientIp, ws: WebSocketUpgrade) -> Response {
    let permit = match st.feeds.admit_socket(ip) {
        Ok(p) => p,                                   // RAII: dropping it frees the per-IP slot
        Err(limit) => return api_error(StatusCode::TOO_MANY_REQUESTS, &limit.to_string()),
    };
    ws.max_message_size(4096).on_upgrade(move |socket| serve_socket(socket, st, permit))
}

// inside serve_socket, per text message:
match parse_subscribe(&text) {
    Err(SubscribeError::CloseSocket) => break,       // Python parity: bad JSON kills the socket
    Err(e) => out.send(e.frame()).await?,            // "Unknown timeframe X", socket stays open
    Ok(want) => {
        current.take().map(|h| h.abort());           // a new message replaces the stream
        let (hub, out) = (st.feeds.clone(), out_tx.clone());
        current = Some(tokio::spawn(async move {
            match hub.subscribe(want).await {        // history fetch, capacity, join, seed: one call
                Err(refusal) => { let _ = out.send(refusal.frame()).await; }
                Ok(mut sub) => {
                    if out.send(sub.history()).await.is_err() { return; }
                    while let Some(frame) = sub.next().await {   // skips Lagged, ends on close
                        if out.send(frame).await.is_err() { return; }
                    }
                }                                    // sub dropped here or on abort: leave()
            }
        }).abort_handle());
    }
}
```

Call site 2, `/api/bars` and the ws history share one cache (`routes/market.rs`):

```rust
async fn bars(State(st): State<AppState>, Query(p): Query<Params>) -> Response {
    let q = match BarsQuery::from_params(&p) {
        Ok(q) => q,
        Err(e) => return api_error(StatusCode::BAD_GATEWAY, &e.to_string()),   // "'M2'" parity
    };
    match st.bars.get(&q, Freshness::Cached).await {
        Ok(bars) => json_ok(&*bars),                                           // bare array, oldest first
        Err(e) => api_error(StatusCode::BAD_GATEWAY, &e.to_string()),
    }
}
```

Call site 3, the feedlog timer in 01-03 (`feedlog.rs`), the only place that turns hub numbers into log lines
and `feed.*` problems:

```rust
let mut log = FeedLog::new(Instant::now());
loop {
    tick.tick().await;                                         // every 2 s
    let view = st.feeds.view(log.wants_lags(Instant::now()));  // copies out of the lock, then unlocks
    let out = log.step(FeedLogInput {
        now: Instant::now(), feeds: &view, status: &st.catalog.status(), bars: &st.bars.stats(),
    });
    for line in out.lines { st.logs.push(line); }
    st.logs.replace_problems("feed.", out.problems);           // derive, never set and forget
}
```

## Shape

**Data structures first.** Four structures carry the whole phase.

1. `HubState` (`feeds/state.rs`), a plain struct with no I/O: `upstreams: HashMap<UpstreamKey, Upstream>`,
   `sockets_per_ip: HashMap<IpAddr, u16>`, monotonic per-provider `FeedTotals`, and the limits. An `Upstream`
   holds its `FeedRules`, its `FeedHealth` (last 300 lags), an `idle_since`, a `oneshot::Sender<()>` stop
   handle, and `series: BTreeMap<Tf, Series>`. A `Series` holds `Option<BarBuilder>` (None until seeded), a
   `Throttle`, a viewer count, a `broadcast::Sender<Frame>` and `last_data`. Every invariant of D-002 lives in
   this one struct, behind one `std::sync::Mutex` that is never held across an `.await`: upstream count at most
   `FEED_MAX_SYMBOLS`, sockets per IP at most 24, an upstream lives while it has viewers or is within 60 s of
   its last viewer leaving, and one builder per `(upstream, tf)`.
2. `BarBuilder` (`market/candles.rs`), the bar rule: bucket `ts - ts % span`, a later bucket opens a bar, the
   same bucket updates, an earlier one merges (Binance, Deriv) or drops (Bybit) per `LatePolicy`.
3. `BarsCache` (`bars_cache.rs`): an LRU of 512 slots keyed by a normalized `BarsQuery`, each slot an
   `Arc<tokio::sync::OnceCell<..>>`, which makes it single-flight for free. TTL 5 s without `to`, 10 min with
   it. `/api/bars` and every ws join read history through it, so N viewers joining the same chart make one
   REST call.
4. `LogRing` (`logbus.rs`), the 12,000-row ring with sequence ids, the `(src, msg)` throttle table, `sources`
   and `problems`. Plain struct, `push(now, Line)`, `snapshot(now, since, brief)`.

**Traced access patterns.**

- *A tick arrives* (hot path). The upstream task reads a frame, calls the pure `parse_frame` outside the lock
  and gets `Parsed::Batch(TickBatch)`. It takes the lock and calls `HubState::ingest(key, epoch, &batch,
  now)`, which records one health sample per frame (Python's `record_tick` granularity: per trade for
  Binance, per message for Bybit, per tick for Deriv). For each series it applies every tick to the builder,
  asks the throttle, and pushes an `Emit { tx, msg }` when admitted. It then unlocks. Serialization happens
  once per emit, outside the lock, and `tx.send(frame)` fans out to every viewer of that `(source, symbol,
  tf)`. Cost per frame is one map lookup and at most 7 builder updates under the lock, plus one
  serialization per admitted series. It does not depend on the viewer count.
- *A viewer subscribes.* `FeedHub::subscribe(want)` (1) peeks under the lock whether the series is already
  seeded; (2) unlocks and fetches history through `BarsCache` (`Freshness::Fresh` when unseeded, so the
  builder's forming bar starts from a fresh snapshot, otherwise `Cached`); on error it returns
  `Refusal::History("Binance history failed: ...")` with Python's per-source text, and no upstream has been
  created, so an unknown symbol never holds a capacity slot; (3) locks and calls `HubState::join`, which
  checks capacity (evicting the longest-idle upstream if one is in grace), creates the upstream entry if it
  is missing (returning an `UpstreamStart` that the shell spawns after unlocking), creates or reuses the
  series, seeds its builder from the last history bar if it is still unseeded (idempotent: the first seed
  wins), subscribes a `broadcast::Receiver` and increments viewers. The receiver is created under the same
  lock as the seed, so no update falls between them.
- *A viewer leaves* (socket closed, new subscribe message, task aborted). `Subscription` is an RAII guard;
  its `Drop` takes the lock and calls `HubState::leave(key, epoch, tf, now)`. When the series' viewer count
  reaches 0 the series is removed; when the upstream has no series, `idle_since = Some(now)`. Leaving twice
  is impossible (Drop runs once), and a stale epoch is a no-op.
- *Grace expires.* No per-leave timers. A sweeper task calls `HubState::sweep(now)` every second; it removes
  every upstream whose `idle_since + 60 s <= now`. Removing the entry drops its `oneshot::Sender`, which ends
  the upstream task. **The map entry's existence is the upstream's life**: there is no separate "running"
  flag to keep in sync, per encode-lessons-in-structure. A rejoin within grace clears `idle_since`.
- *`/api/health` and feedlog read.* `FeedHub::view(detail)` copies counters, ages and (when asked) lags out
  under the lock and returns a `FeedView`. `/api/health` is a pure function of `FeedView`. Feedlog keeps the
  previous totals itself and diffs them, so the hub's counters are monotonic and have one writer. The Python
  `_logged` mutation of `FEED_HEALTH` by the feedlog thread (`feedlog.py:123-124`) is a second writer that
  this shape removes, per separate-before-serializing-shared-state.

**One lock, measured where it could fail.** All feed state sits behind one mutex. The critical sections are
small and synchronous: a map lookup, at most 7 builder updates, a ring push of 300 lags. The fan-out
`send`, serialization, history fetch, socket I/O and spawning are all outside it. My estimate of a few
microseconds per frame is not measured. If profiling ever shows contention, the split is mechanical: one
mutex per `Upstream` beneath a registry mutex, with the same pure methods. The shape does not depend on
the single lock; the tests do not see it.

**Where the pure shape stops.** Four places are irreducibly async and stay in the shell, each kept to one
function: the upstream session loop (`feeds/upstream.rs::run`, connect, keepalive, reconnect with `Backoff`),
the Deriv RPC pool (`req_id` matching over pooled sockets, `sources/deriv.rs::RpcPool`), the single-flight
cache (`BarsCache::get`), and the socket loop (`routes/ws.rs::serve_socket`, ping every 30 s, close after
60 s without a pong). Even there the decisions are pure: `Backoff::next(unit)` takes the random number as an
argument, `LivePlan` is data (url, hello frames, keepalive kind, parser), and the Deriv deep-history window
planner is a pure iterator.

**The three adapters are data plus one pure parser, not a trait.** Each provider module exposes
`live_plan(&UpstreamKey, &Endpoints) -> LivePlan`, `parse_frame(&str) -> Parsed`, `rules() -> FeedRules`, and
the 01-01 REST half (`history`, `symbols`). One shell loop runs all three. Bybit's JSON `{"op":"ping"}` every
20 s and Binance and Deriv's protocol pings are a `KeepAlive` value, not code paths. Deriv's error reply is
`Parsed::EndSession { status: "Deriv: <msg>" }`. Adding a provider means adding one module and one match arm
in `sources/mod.rs`.

**Validation at boundaries, trust inside** (boundary-discipline). `Ticker` is a validated newtype
(`[A-Za-z0-9_.\-]{1,40}`), since symbols are pasted into upstream URLs and the Python code never checked them.
`Tf` and `Source` are enums parsed once (`Tf::parse`, `Source::from_wire` with the Binance fallback).
`UpstreamKey::new` normalizes the identity (Binance `BTCUSD` and `BTCUSDT` are one `@aggTrade` stream, keyed
`binance:BTCUSDT`). Upstream JSON is parsed into `Tick` by private serde DTOs inside each provider module; no
wire type of an upstream reaches the public surface. The browser wire types live in `hub-types` and appear
only in route handlers and in `ServerMsg` frames.

**Logging without coupling 01-02 to 01-03.** 01-01 and 01-02 log through `tracing` with an explicit target
equal to the log source (`"server"`, `"feed.binance"`) and an optional `every = 30` field. In 01-03,
`RingLayer` is a `tracing_subscriber::Layer` that maps target to `src`, level to `lvl`, the message plus
other fields to `msg` (Python's ` k=v` facts), and honours `every`, `persist`, `alive`. No 01-01 or 01-02 code
names `LogBus`. Event-like lines are written where they happen (`"BTCUSD M1: chart stream opened"` when a series
is created, `"Bybit reconnecting: ..."` once per upstream, not per viewer). Aggregates (the 60 s summary),
silence (120 s per series) and the `feed.*` problems are derived every 2 s by the pure `FeedLog::step` from
`FeedView`, `StatusResponse` and the cache's served-bar totals. `LogRing::replace_problems("feed.", desired)`
replaces the whole prefix each tick, so a stale problem cannot survive (derive, not sync).

**Interface depth.** The public surface of the feed side is: `FeedHub::admit_socket`, `FeedHub::subscribe`,
`Subscription::{history, next}`, `FeedHub::{health, view, spawn_sweeper}`. Behind it sit: history fetch and
single-flight, capacity and eviction, per-IP counting, upstream spawn and stop, seeding, the three providers'
parsers and keepalive rules, reconnect backoff, the three late-tick policies, throttling, health stats, and
the fan-out. The route never sees an `UpstreamKey`'s epoch, a builder or a broadcast channel. `BarsCache`
exposes `get(q, freshness)` and `stats()`; TTL, normalization (Binance and Bybit cap `limit` at 1000) and
single-flight are hidden.

**What the system deliberately does not do.** No actors, no per-viewer builders, no timers per leave, no
`/api/status` cache (the status is an O(1) read of the catalog's two `watch` values), no `HUB_TAIL_FILES`, no
stdout `Tee` (the fmt layer prints message-only lines so the start-up lines stay byte-identical), no
silence-by-expectation (`expect()` is never called in Python; the snapshot keeps the `expect: null`,
`state: "ok"` shape), no `/api/restart`.

**`AppState` grows additively.** 01-01 defines `AppState { cfg, catalog, bars }`; 01-02 adds `clock, feeds`;
01-03 adds `logs`; Phase 3 adds `db`. Every field is a cheap `Clone` (an `Arc` inside). The rate limiter is
a tower layer built in `app()`, not state.

## Synthesis decision

Arena of three runners (all `inherit-parent` per `~/.agents/pstack-models.md`), each seeded with a different
structural direction: A actor per upstream plus a registry task, B pure core behind one lock (this file),
C a `Source` trait object per provider. One read-only cross-judge scored them on six criteria
(contract fidelity, FeedHub concurrency, interface depth, plan boundaries, extension cost, observability):
A 23, B 26, C 23. The orchestrator's own reading agreed. No candidate violated a prohibition. No dropouts.

**Base: B.** It checks every D-002 invariant (capacity, 24 sockets per IP, 60 s grace, one builder per
timeframe, eviction) atomically in one synchronous method that tests drive with explicit `Instant`s; it
fetches history before `join`, so a bad symbol never holds a capacity slot; the map entry's existence is the
upstream's life; every counter has one writer; it keeps the exact `feed.<src>:<SYM TF>` problem key and
Python's `bars_served` meaning; and its plan split compiles and tests alone, with the contract suite served
in-process from 01-01.

**Convergence (all three candidates, independently):** one upstream per `(source, resolved symbol)` with a
builder per timeframe; viewer count derived from the broadcast channel or the series count, never synced;
one single-flight `BarsCache` shared by `/api/bars` and the websocket history; logging through `tracing`
fields until 01-03's layer reads them; `mt5: "off"` and `arm: "off"` kept in `/api/symbols` (`lab.js:95-97`
warns otherwise); the ws `history` message keeps `source`, `symbol`, `tf`; no `/api/status` cache; Binance
health keyed by the mapped symbol; every provider reconnects with jittered backoff; the contract suite runs
the Rust router in-process until the binary exists.

**Grafted:**
1. From A, `Joined.notice`: a viewer who joins while the upstream link is down receives the current
   `status` frame right after `history`, so late viewers see what earlier ones saw. In B this is a
   `last_notice: Option<Frame>` on `Upstream`, set and cleared by the session loop, read in `join`.
2. From A, closed-bar publish: when a tick opens a new bucket, the closed bar's final state is sent once
   before the new bar, bypassing the throttle. It fixes Python's lost-close (50 ms throttle with no trailing
   flush, `server.py:365`) without a new message type. It changes the frame count against Python, so it is
   recorded as decision D-004, and the contract suite compares message types and field sets, not counts.
3. From A, the untagged `SymbolEntry` enum (one shape per source: Deriv with `digits` and `open`, Bybit with
   `digits`, Binance without) replaces B's `Symbol` with optional fields, so no field is optional where
   Python always sends it.
4. From C, `HEAD` handled by the same static rule (`HEAD /.env` is 404; the Python stdlib leak is not copied)
   and a `FakeSource`-style test seam: B's `Endpoints` already points every provider at local fakes; C's
   scripted provider events are adopted as the fixture format for `ws_e2e.rs`.

**Fixes to the base (from the judge's fact check):**
1. `Ticker` validation runs after the Binance normalization `bsym_of` (`split(":")[-1]`, `server.py:53-57`),
   so `EXCHANGE:SYM` inputs keep working; the newtype then validates `[A-Za-z0-9_.\-]{1,40}`.
2. A series is removed when its last viewer leaves, so its builder goes with it; a rejoin inside the grace
   window re-seeds from `BarsCache` (normally a hit). The test is named
   `rejoin_within_grace_reuses_upstream_and_reseeds_series`.
3. The `_logged` second writer citation is `feedlog.py:123-124`.

**Rejected:**
- C's `Arc<dyn Source>` trait object now: Phase 1 has exactly three compile-time providers, and B's
  "one module plus one match arm" costs less today. Kept as Phase 7 open question Q-ARCH-1 below.
- A's actor and registry tasks: they add command enums and snapshot round trips (400 per feed-log step)
  that hide less than B's one lock; A's own "registry as a mutex" alternative is what B is.
- A's per-IP refusal by error frame: refused with HTTP 429 before the upgrade instead (B and C), so the
  wire protocol stays untouched.
- C's `errors` redefinition ("sessions that ended without data"): B keeps Python's meaning (error frames,
  failed HTTP bars, stream deaths).

**Open questions carried into the plans (defaults written, flagged at approval):**
- Q-ARCH-1 (Phase 7): MT5 bridges fit this hub only if the bridge emits ticks; a bar-push bridge needs a
  bar lane in `feeds/`. Decide when Phase 7 is planned; the trait-object seam from C is the fallback.
- Q-ARCH-2: in local mode (`AUTH_ENABLED=false`) every tab is 127.0.0.1, so all tabs share one 24-socket
  budget. Default: loopback peers are exempt from the per-IP socket limit while auth is off.
- Q-ARCH-3: at `FEED_MAX_SYMBOLS`, the longest-idle upstream in grace is evicted before refusing (default).
- Q-ARCH-4: Deriv `/api/bars` `limit` stays uncapped in Phase 1 (parity); a cap is a Phase 4 hardening item.
- Q-ARCH-5: the single lock is estimated, not measured: a replay load test (recorded Binance frames at 10x
  into 400 upstreams) is a 01-02 acceptance check.

## Tradeoffs accepted

- We accept one mutex over all feed state in exchange for every D-002 invariant (capacity, per-IP, grace,
  eviction, one builder per tf) being checked atomically in one synchronous method that unit tests drive
  with explicit `Instant`s and no runtime.
- We accept a per-series throttle in place of Python's per-viewer one in exchange for one serialization per
  update regardless of viewer count. Viewers of one series now see identical frames; Python's were
  identical up to the phase of each viewer's 50 ms window.
- We accept no trailing flush after a throttled burst (Python parity: the last trade of a burst waits for
  the next trade) in exchange for no timer in the hot path. The pure `Throttle` makes adding one later a
  local change.
- We accept that ticks arriving for a timeframe before its builder is seeded are dropped in exchange for
  never double-counting volume between the REST snapshot and the live stream. Python loses the same window
  (between its REST call and its upstream connect).
- We accept that a slow viewer whose broadcast receiver lags skips frames (`RecvError::Lagged` is
  ignored) in exchange for never blocking the upstream. Each `bar` frame is a full snapshot of the
  current bar, so only the final update of a just-closed bar can be lost; Vela already gap-fills over HTTP.
- We accept that `/api/health` lists only live upstreams (health lives in the `Upstream` entry and goes
  with it) and keys Binance by the mapped symbol (`binance:PAXGUSDT` for a `XAUUSD` request) in exchange for
  one source of truth and no unbounded map. No page reads `/api/health`.
- We accept a uniform rule for an empty history (the builder opens on the first tick) in place of three
  per-source behaviours (Binance dies, Bybit stops, Deriv continues) in exchange for one code path. The
  contract suite cannot see the difference.
- We accept that Binance now reconnects with backoff and sends `status "Binance reconnecting: <e>"`
  (Python ended the stream silently) because the spec asks for backoff on every provider.
- We accept the 1 s granularity of the grace sweep (an upstream lives 60 to 61 s after its last viewer) in
  exchange for zero timers keyed to leave events and a fully deterministic `sweep(now)`.
- We accept that `hub-types` gains one file per plan (`market.rs`, `ws.rs`, `health.rs`, `logs.rs`) in
  exchange for each plan compiling alone; the crate is still the one Rust model of the contract.
- We accept that each provider file is edited by two plans (REST half in 01-01, live half in 01-02) in
  exchange for grouping by provider knowledge rather than by plan (avoiding temporal decomposition).

## Alternatives considered

- **Actor per upstream** (the task owns its builders; viewers send `Join { tf, reply }` over an mpsc). It
  hides the lock, but capacity, per-IP and eviction span upstreams, so a registry actor is needed anyway,
  join becomes request and reply across two actors, and every test needs a runtime and timing. It exposes
  more surface (command enums, reply channels) and hides less. Lost on interface depth and testability.
- **Broadcast raw ticks, keep a builder per viewer** (the closest port of Python). It is the smallest diff
  from `server.py` and keeps per-viewer semantics exactly, but it keeps N builders, N throttles and N
  serializations per tick, and feedlog counting would still be per viewer unless moved. It hides nothing
  that the chosen shape does not, and costs scale with viewers. Lost on the D-002 requirement itself
  (a builder per timeframe, not per viewer).
- **Per-upstream mutex in a concurrent map** (DashMap style registry). Less contention, but the capacity and
  eviction checks need a global view, which brings back a global lock plus a lock-ordering rule. Kept as the
  documented escape hatch, not the base.

## Open questions and risks

- Spec item 3 and D-001 say `/api/symbols` drops `mt5` and `arm`, but `hub/lab.js:96-97` warns in the console
  unless each is `"ok"` or `"off"` (an absent key is `undefined`), and the browser edit is limited to
  `wsUrl()`. This design keeps both keys with the constant value `"off"` (the `Off` unit type), which is
  exactly what Python sends with MT5 and arm disabled (`server.py:747`, `129`). Do you accept that over the
  spec text, and should item 12's removal of `LabMT5Status` and `LabARMStatus` wait for a later phase?
- How should the 25th socket from one IP be refused? This design answers HTTP 429 before the upgrade, which
  leaves the wire protocol untouched (D-002 gives `kind` only to the global cap). The browser then shows
  "Feed offline". Is that acceptable, or should it upgrade and send `error` with a new `kind`?
- At `FEED_MAX_SYMBOLS`, should an idle upstream in grace be evicted to admit a new one (this design does,
  longest idle first), or should grace upstreams count as occupied?
- `ClientIp` (the `CF-Connecting-IP` when the peer is loopback rule) is needed by the per-IP socket limit in
  01-02, so this design moves `net::client_ip` into 01-02 and leaves the rate limiter, headers and POST guard
  in 01-03. Is that deviation from the plan list acceptable?
- The contract suite in 01-01 runs the Rust side in-process (`hub_core::serve` on `127.0.0.1:0`) when
  `CONTRACT_RS_URL` is unset, because the binary arrives in 01-03; CI in 01-03 points it at the binary.
  `CONTRACT_PY_URL` unset still skips with a printed reason. Is that compatible with the G1 gate wording?
- Python's Deriv symbol list never refreshes after the first success, so the `open` flag goes stale. Port
  that, or refresh hourly like Bybit?
- `/api/bars` for Deriv accepts any `limit` today (parallel windows of 1,000). Should Rust cap it (for
  example 20,000) as a cheap abuse limit, given the rate limiter only counts requests?
- Risk: the single lock is estimated, not measured. A load test with recorded Binance BTCUSDT and ETHUSDT
  frames replayed at 10x into 400 upstreams should be a 01-02 acceptance check.
- Risk: `tokio::sync::OnceCell::get_or_try_init` lets each waiter retry after a failed init, so during an
  upstream outage N concurrent joiners make up to N sequential REST calls. Acceptable under the per-IP and
  rate limits, but worth a counter.
- Risk: Python's `round()` is banker's rounding and Rust's `f64::round` rounds half away from zero, so
  `age_ms` and lag values can differ by 1 on a tie. Values are not compared by the contract suite.

## Next implementation step

Write `hub-types/src/market.rs` and `hub-core/src/static_files.rs::resolve` with the probe table
(`%2e%2e/`, `%5c`, `.env`, `vendor/a/b.js`, `server.py`, `HEAD /.env`) as unit tests, then `app()` with the
static fallback, so the first contract case runs on the first day.

---

## Module map

```text
Cargo.toml                         workspace; [workspace.dependencies] one version each      01-01
rust-toolchain.toml                exact stable, clippy, rustfmt                             01-01
crates/
  hub-types/src/
    lib.rs                         pub mod market, ws, health, logs                          01-01 (+1 line per plan)
    market.rs                      Source, Tf, Bar, Symbol, SymbolGroup, SymbolsResponse,
                                   Off, StatusResponse, FeedStatus, InstrumentsResponse,
                                   Broker, Instrument, Variant, ErrorBody                    01-01
    ws.rs                          Subscribe, ServerMsg, ErrorKind                           01-02
    health.rs                      HealthResponse, FeedHealthEntry                           01-02
    logs.rs                        Level, LogRow, SourceInfo, LogSnapshot, LogsResponse      01-03
  hub-core/src/
    lib.rs                         app(), serve(), AppState, BackgroundTasks                 01-01 (fields added 01-02, 01-03)
    config.rs                      Config, AuthMode, ConfigError                             01-01
    static_files.rs                resolve() [pure], serve()                                 01-01
    catalog.rs                     Catalog (watch per provider), refresh loops, status()     01-01
    bars_cache.rs                  BarsCache, HistoryFetch, Freshness, ServeStats            01-01
    market/                        pure, no I/O, no clock reads
      mod.rs                       Ticker, PerSource<T>                                      01-01
      bars_query.rs                BarsQuery, BarsQueryError                                 01-01
      instruments.rs               instrument_key, variant_rank, build                       01-01
      candles.rs                   Tick, TickBatch, BarBuilder, LatePolicy, Throttle,
                                   FeedRules                                                 01-02
      feed_health.rs               FeedHealth                                                01-02
      backoff.rs                   Backoff                                                   01-02
    sources/                       one file per provider: all of that provider's knowledge
      mod.rs                       Sources, Endpoints, FeedError, history() dispatch         01-01
                                   UpstreamKey, LivePlan, KeepAlive, Parsed, live_plan()     01-02
      binance.rs                   bsym_of, CRYPTO, klines parse, history                    01-01
                                   aggTrade parse_frame, rules, live plan                    01-02
      bybit.rs                     BybitTicker::parse, symbols, history                      01-01
                                   publicTrade / tickers parse_frame, rules, live plan       01-02
      deriv.rs                     RpcPool, active_symbols, WindowPlan [pure], history       01-01
                                   ticks parse_frame, rules, live plan                       01-02
    clock.rs                       Clock, Now, sntp_offset [pure], measure()                 01-02
    feeds/
      mod.rs                       pub use FeedHub, Subscription, SocketPermit, Refusal     01-02
      state.rs                     HubState [pure], Upstream, Series, Emit, FeedView         01-02
      hub.rs                       FeedHub shell: Mutex<HubState>, subscribe, sweeper        01-02
      upstream.rs                  run(): session loop, keepalive, backoff                   01-02
    routes/
      mod.rs                       api_error, json_ok, NO_STORE layer, 404 fallbacks          01-01
      market.rs                    /api/status, /api/symbols, /api/instruments, /api/bars    01-01
      ws.rs                        /ws, parse_subscribe [pure], serve_socket                 01-02
      health.rs                    /api/health                                               01-02
      logs.rs                      /api/logs, POST /api/logs/journal                         01-03
    net.rs                         client_ip [pure], ClientIp extractor                      01-02
                                   ClientIpKey, rate_limit(), security_headers(), post_guard 01-03
    logbus.rs                      LogRing [pure], redact [pure], file line format,
                                   LogBus, RingLayer, FileSink                               01-03
    feedlog.rs                     FeedLog [pure step], spawn()                              01-03
  hub-core/tests/
    contract.rs                    two-server shape comparison, in-process Rust by default   01-01 (cases added 01-02, 01-03)
    contract/requests.toml         cases + allowed differences                               01-01 (+01-02, 01-03)
    fixtures/*.jsonl               recorded upstream frames per provider                     01-02
    ws_e2e.rs                      /ws against local fake upstreams (Endpoints override)    01-02
  hub-server/src/main.rs           dotenv, tracing, build, bind, start-up lines              01-03
hub/lab.js                         wsUrl() only                                              01-03
hub/hub.html                       restartChart() arm branch removed                         01-03
Procfile, scripts/dev.sh, .github/workflows/ci.yml, .gitignore, AGENTS.md                    01-03 (.gitignore 01-01)
```

Call chain for a live bar, three files: `feeds/upstream.rs` (frame) -> `sources/<provider>.rs::parse_frame`
-> `feeds/state.rs::ingest` (-> `market/candles.rs`). Call chain for a subscribe, three files:
`routes/ws.rs` -> `feeds/hub.rs::subscribe` -> `feeds/state.rs::join` (history via `bars_cache.rs`).

---

## Type and signature sketch

### hub-types (01-01: market.rs)

```rust
// crates/hub-types/src/market.rs
//! The Rust model of the browser contract. Not the oracle: the contract suite compares raw JSON
//! between the two servers. `deny_unknown_fields` is on every type that has no `#[serde(flatten)]`
//! (serde does not support the two together).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The three free feeds. Unknown or missing sources fall back to Binance on both `/ws` and
/// `/api/bars` (server.py:697, 979); `from_wire` is the one place that rule lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source { Deriv, Bybit, Binance }

impl Source {
    pub const ALL: [Source; 3] = [Source::Deriv, Source::Bybit, Source::Binance];
    /// `None`, `"mt5"`, `"arm"` and any unknown string become `Binance` (Python parity).
    pub fn from_wire(s: Option<&str>) -> Source { todo!() }
    pub fn as_str(self) -> &'static str { todo!() }
    /// Display name used in error texts and `/api/instruments` brokers: "Deriv", "Bybit", "Binance".
    pub fn display(self) -> &'static str { todo!() }
}

/// The only valid timeframes (server.py:49). Serialized as "M1".."D1".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tf { M1, M5, M15, M30, H1, H4, D1 }

impl Tf {
    pub const ALL: [Tf; 7] = [Tf::M1, Tf::M5, Tf::M15, Tf::M30, Tf::H1, Tf::H4, Tf::D1];
    pub fn parse(s: &str) -> Option<Tf> { todo!() }
    /// 60, 300, 900, 1800, 3600, 14400, 86400.
    pub fn secs(self) -> i64 { todo!() }
    pub fn as_str(self) -> &'static str { todo!() }
}

/// One candle. `time` is epoch seconds of the bar open. Deriv volume is always 0.0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bar { pub time: i64, pub open: f64, pub high: f64, pub low: f64, pub close: f64, pub volume: f64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolGroup {
    Commodity, Forex, Indices, Crypto,
    BybitMetal, BybitPerp, BybitSpot, BybitFutures, BybitInverse, BybitMark, BybitIndex,
}

/// One `/api/symbols` entry. The field set differs per source (Deriv: digits + open; Bybit: digits;
/// Binance: neither), so the optional fields are skipped when absent and only the three
/// constructors below build a `Symbol`. Fields are pub for deserializing Python output in tests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Symbol {
    pub ticker: String,
    pub name: String,
    pub source: Source,
    pub group: SymbolGroup,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digits: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
}

impl Symbol {
    pub fn deriv(ticker: String, name: String, group: SymbolGroup, digits: u32, open: bool) -> Symbol { todo!() }
    pub fn bybit(ticker: String, name: String, group: SymbolGroup, digits: u32) -> Symbol { todo!() }
    pub fn binance(ticker: &str, name: &str) -> Symbol { todo!() }
}

/// Serializes as the string "off". Phase 1 has no MT5 and no arm (D-001), and lab.js:96-97 warns
/// unless the value is "ok" or "off", so the keys stay with the only value that exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Off;
// impl Serialize for Off -> "off"; impl Deserialize accepts only "off".

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolsResponse {
    /// Order: bybit, deriv, binance (Python: arm, bybit, deriv, mt5, binance; arm and mt5 are empty).
    pub symbols: Vec<Symbol>,
    pub mt5: Off,
    pub arm: Off,
    pub deriv: usize,
    pub bybit: usize,
    /// The HTTP port (one port now). Kept one phase so a page loaded from Python still works (D-001).
    pub ws_port: u16,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedStatus { pub ok: bool, pub msg: String }

/// `ok` means "symbol list loaded", not "feed live" (server.py:863-882). No arm, no mt5 (D-001).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusResponse { pub deriv: FeedStatus, pub bybit: FeedStatus, pub binance: FeedStatus }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentsResponse { pub brokers: Vec<Broker>, pub instruments: Vec<Instrument> }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Broker { pub id: Source, pub name: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instrument {
    pub key: String,
    pub name: String,
    /// The first contributing symbol's group string, as Python does (not re-ranked).
    pub group: String,
    pub brokers: BTreeMap<Source, Vec<Variant>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant { pub ticker: String, pub label: String }

/// Every API error body: 404 `{"error":"not found"}`, 502 `{"error": "<text>"}`, 403, 413, 415, 429.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody { pub error: String }
```

### hub-types (01-02: ws.rs, health.rs)

```rust
// crates/hub-types/src/ws.rs
//! The D-002 protocol, unchanged except the optional `kind` on `error`.

/// The client's one untyped message. No `deny_unknown_fields`: Python ignores extra keys.
/// Missing fields default at the parse boundary (routes/ws.rs), not here.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Subscribe {
    #[serde(default)] pub source: Option<String>,
    #[serde(default)] pub symbol: Option<String>,
    #[serde(default)] pub tf: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum ServerMsg {
    /// `symbol` is the mapped Binance symbol (PAXGUSDT) and the request echo for Bybit and Deriv.
    History { bars: Vec<Bar>, source: Source, symbol: String, tf: Tf },
    Bar {
        bar: Bar,
        /// Local wall clock, uncorrected (server.py:367).
        sent: i64,
        /// Present for Binance and Bybit, absent for Deriv.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trade_ms: Option<i64>,
        /// Corrected clock minus source time; always present, `null` when the source has no time.
        lag_ms: Option<f64>,
    },
    Status { msg: String },
    Error {
        msg: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ErrorKind>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind { Capacity }

// crates/hub-types/src/health.rs
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthResponse {
    pub clock_offset_ms: i64,
    /// Key "<source>:<upstream ident>", live upstreams only.
    pub feeds: BTreeMap<String, FeedHealthEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedHealthEntry {
    pub ticks: u64,
    pub age_ms: i64,
    pub lag_median_ms: Option<i64>,
    pub lag_p95_ms: Option<i64>,
}
```

### hub-types (01-03: logs.rs)

```rust
// crates/hub-types/src/logs.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level { Debug, Info, Warn, Error }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogRow { pub id: u64, pub t: i64, pub src: String, pub lvl: Level, pub msg: String }

/// Shape kept from logbus.py; `expect` is always null and `state` always "ok" in Phase 1
/// (silence-by-expectation is dead code in Python and is not ported).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInfo { pub last: i64, pub expect: Option<u64>, pub state: String, pub note: String }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogSnapshot {
    pub boot: i64,
    pub last: u64,
    pub now: i64,
    pub errors_15m: usize,
    pub latest_error: Option<LogRow>,
    pub problems: BTreeMap<String, String>,
    pub sources: BTreeMap<String, SourceInfo>,
    /// Absent when `brief`; at most 3,000 rows, ids greater than `since`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<LogRow>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogsResponse {
    #[serde(flatten)] pub snapshot: LogSnapshot,
    /// Always false; log_panel.js never reads it, but the key is in the contract.
    pub journal_raw: bool,
}
```

### hub-core: lib.rs and config.rs (01-01, fields added later)

```rust
// crates/hub-core/src/lib.rs
pub mod config;
mod static_files; mod catalog; mod bars_cache; pub mod market; pub mod sources; mod routes;
// 01-02: pub mod clock; pub mod feeds; mod net;
// 01-03: pub mod logbus; mod feedlog;

pub use config::Config;

/// Shared state. Every field is a cheap clone. Plans add fields; nothing is ever removed.
/// Phase 3 adds `db` (and sessions) here without touching any route signature.
#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub catalog: catalog::Catalog,
    pub bars: bars_cache::BarsCache,
    // 01-02:
    pub clock: clock::Clock,
    pub feeds: feeds::FeedHub,
    // 01-03:
    pub logs: logbus::LogBus,
}

impl AppState {
    /// No network I/O: an empty catalog, an empty cache, an unmeasured clock (offset 0).
    /// Tests call this directly. `build_with` lets tests replace upstream endpoints with local fakes.
    pub async fn build(cfg: Config) -> Result<AppState, StartError> { todo!() }
    pub async fn build_with(cfg: Config, endpoints: sources::Endpoints) -> Result<AppState, StartError> { todo!() }
    /// 01-03: the binary passes the Clock and LogBus it created before installing tracing, so the
    /// ring layer and the state share one bus. `build` creates both itself (log dir from cfg).
    pub async fn build_parts(cfg: Config, clock: clock::Clock, logs: logbus::LogBus) -> Result<AppState, StartError> { todo!() }

    /// Starts every background task and returns their handles (dropping the value does not stop them;
    /// `shutdown()` aborts them). 01-01: Deriv and Bybit symbol refresh. 01-02: sweeper, SNTP.
    /// 01-03: feedlog.
    pub fn spawn_background(&self) -> BackgroundTasks { todo!() }
}

pub struct BackgroundTasks { handles: Vec<tokio::task::JoinHandle<()>> }
impl BackgroundTasks { pub fn shutdown(self) { todo!() } }

/// The only router. hub-server and the contract suite both serve exactly this.
pub fn app(state: AppState) -> axum::Router {
    // TODO:
    // Router::new()
    //   .merge(routes::market::routes())      01-01
    //   .merge(routes::ws::routes())          01-02
    //   .merge(routes::health::routes())      01-02
    //   .merge(routes::logs::routes())        01-03
    //   .layer(no_store on /api/*)            01-01 (routes::mod)
    //   .fallback(static_files::serve)        01-01, also serves HEAD through the same check
    //   .method_not_allowed_fallback(not_found)  POST to a GET route is 404, as Python
    //   .layer(net::rate_limit())  /api/* only 01-03
    //   .layer(net::security_headers())       01-03
    //   .with_state(state)
    todo!()
}

/// `app(state)` with `ConnectInfo<SocketAddr>` (needed by ClientIp). The single place that turns
/// the router into a service, so the binary and the contract harness cannot drift.
pub async fn serve(listener: tokio::net::TcpListener, state: AppState) -> std::io::Result<()> { todo!() }

#[derive(Debug, thiserror::Error)]
pub enum StartError { #[error(transparent)] Config(#[from] config::ConfigError), #[error("{0}")] Io(#[from] std::io::Error) }

// crates/hub-core/src/config.rs
/// Read once at start. The only code that reads env vars (dotenv loading is hub-server's job).
#[derive(Clone, Debug)]
pub struct Config {
    pub http_port: u16,              // HUB_HTTP_PORT, default 8770
    pub static_dir: PathBuf,         // HUB_STATIC_DIR, default "hub"
    pub log_dir: PathBuf,            // HUB_LOG_DIR, default "hub/logs"
    pub public_origin: String,       // PUBLIC_ORIGIN, default "http://127.0.0.1:<port>"
    pub auth: AuthMode,              // AUTH_ENABLED, only "false" exists in Phase 1
    pub feed_max_symbols: usize,     // FEED_MAX_SYMBOLS, default 400; counts (source, symbol) upstreams
}

/// Phase 3 adds `On { .. }`. With `Off` the bind address is derived, never read: there is no
/// setting that can bind anything but 127.0.0.1 (D-001), so the "refuse other binds" rule is a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthMode { Off }

/// Fixed by D-002; constants, not env.
pub const WS_PER_IP: u16 = 24;
pub const GRACE: Duration = Duration::from_secs(60);

impl Config {
    pub fn from_env() -> Result<Config, ConfigError> { Self::from_lookup(|k| std::env::var(k).ok()) }
    /// Pure: tests pass a closure over a map.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        // TODO: AUTH_ENABLED other than unset/"false" -> ConfigError::AuthNotAvailable
        //       (auth arrives in Phase 3). Bad numbers -> ConfigError::Invalid { key, value }.
        todo!()
    }
    pub fn bind_addr(&self) -> SocketAddr { todo!() }  // AuthMode::Off => 127.0.0.1:<http_port>
    /// http://localhost:<port>, http://127.0.0.1:<port>, PUBLIC_ORIGIN.
    pub fn allowed_origins(&self) -> [String; 3] { todo!() }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("AUTH_ENABLED={0} is not available before Phase 3; local mode needs AUTH_ENABLED=false")]
    AuthNotAvailable(String),
    #[error("{key}={value} is not valid")]
    Invalid { key: &'static str, value: String },
}
```

### hub-core: static files (01-01)

```rust
// crates/hub-core/src/static_files.rs
const STATIC_EXT: [&str; 8] = [".html", ".js", ".css", ".png", ".svg", ".ico", ".woff", ".woff2"];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StaticTarget {
    /// `/` and `""` -> static_dir/index.html (a meta refresh to hub.html).
    Index,
    /// Relative path under static_dir: one segment, or `vendor/<file>`.
    File { rel: PathBuf, mime: &'static str },
}

/// Pure port of `static_allowed` (server.py:888-898), applied to the path only (axum has already
/// split the query). Percent-decode first; invalid UTF-8 after decoding refuses. Empty segments are
/// dropped as Python does. Refuse any segment starting with '.' and any backslash anywhere. Allow one
/// segment, or exactly two with the first "vendor"; the extension check is case-insensitive.
pub(crate) fn resolve(raw_path: &str) -> Option<StaticTarget> { todo!() }

/// Fallback handler for GET and HEAD (axum routes HEAD through GET, so `HEAD /.env` is 404 too,
/// closing the stdlib leak noted in grounding). Refused -> 404 {"error":"not found"} with no-store;
/// served -> the file with `Cache-Control: no-cache, must-revalidate`. A missing file is the same 404.
pub(crate) async fn serve(State(st): State<AppState>, method: Method, uri: Uri) -> Response { todo!() }
```

### hub-core: market (pure) (01-01 and 01-02)

```rust
// crates/hub-core/src/market/mod.rs
/// A validated upstream symbol: 1 to 40 of [A-Za-z0-9_.-]. Symbols are pasted into upstream URLs
/// and topics, so this is the boundary check Python never had.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Ticker(String);
impl Ticker {
    pub fn parse(s: &str) -> Result<Ticker, BadTicker> { todo!() }
    pub fn as_str(&self) -> &str { &self.0 }
}

/// One value per provider, indexable by Source. Used for counters and stats.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PerSource<T> { pub deriv: T, pub bybit: T, pub binance: T }
impl<T> std::ops::Index<Source> for PerSource<T> { /* .. */ }
impl<T> std::ops::IndexMut<Source> for PerSource<T> { /* .. */ }

// crates/hub-core/src/market/bars_query.rs
/// A parsed, normalized `/api/bars` request; also the BarsCache key. Two queries that hit the same
/// upstream data are equal: Binance symbols are mapped (`BTCUSD` == `BTCUSDT`), and Binance and
/// Bybit `limit` is capped at 1000 before it becomes a key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BarsQuery { pub source: Source, pub symbol: Ticker, pub tf: Tf, pub limit: u32, pub to_ms: Option<i64> }

impl BarsQuery {
    /// Defaults: source Binance (fallback rule), symbol "BTCUSD", tf "M1", limit 1000; empty `to` is None.
    /// Errors render as Python's 502 texts: an unknown tf is `'M2'` (the KeyError repr).
    pub fn from_params(p: &HashMap<String, String>) -> Result<BarsQuery, BarsQueryError> { todo!() }
    /// What a ws join fetches: limit 1000, no `to`.
    pub fn live_seed(source: Source, symbol: &Ticker, tf: Tf) -> BarsQuery { todo!() }
    /// 5 s without `to` (the newest bars move), 10 min with it (closed history).
    pub fn ttl(&self) -> Duration { todo!() }
}

#[derive(Debug, thiserror::Error)]
pub enum BarsQueryError {
    #[error("'{0}'")] UnknownTf(String),
    #[error("invalid limit {0}")] BadLimit(String),
    #[error("invalid to {0}")] BadTo(String),
    #[error("invalid symbol {0}")] BadSymbol(String),
}

// crates/hub-core/src/market/instruments.rs
/// Pure port of `instrument_key` (server.py:799-830) for deriv, bybit and binance:
/// DERIV_INDEX map, frx/cry prefixes; Bybit base + "USD" with a variant label; USDC-quoted and
/// mark/index of dated futures are None.
pub fn instrument_key(s: &Symbol) -> Option<(String, Option<String>)> { todo!() }
pub fn variant_rank(label: &str) -> u8 { todo!() }
/// Input order matters (first name wins): deriv, then bybit, then binance (server.py:833).
/// Sort: broker count desc, group rank, key.
pub fn build(deriv: &[Symbol], bybit: &[Symbol], binance: &[Symbol]) -> InstrumentsResponse { todo!() }

// crates/hub-core/src/market/candles.rs (01-02)
/// One trade or tick, already parsed. `qty` is 0.0 for Deriv ticks and Bybit mark/index.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tick { pub price: f64, pub qty: f64, pub ts_ms: i64 }

/// Everything one upstream frame carried. `src_ms` is the one health sample for the frame
/// (Binance: the trade T, Bybit: the last trade T or the ticker ts, Deriv: epoch * 1000), which keeps
/// Python's record_tick granularity.
#[derive(Clone, Debug, PartialEq)]
pub struct TickBatch { pub ticks: smallvec::SmallVec<[Tick; 4]>, pub src_ms: Option<i64> }

/// What a trade older than the current bar does. Binance and Deriv merge it into the current bar
/// (server.py:356-358, 678-679); Bybit drops it (server.py:473-476). Ported as observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatePolicy { MergeIntoCurrent, Drop }

/// Per-provider rules, pure data, from `sources::<provider>::rules()`.
#[derive(Clone, Copy, Debug)]
pub struct FeedRules {
    pub late: LatePolicy,
    /// Some(50 ms) for Binance and Bybit (20 sends/s, no trailing flush); None for Deriv (every tick).
    pub min_gap: Option<Duration>,
    /// Binance and Bybit put `trade_ms` on `bar`; Deriv does not.
    pub sends_trade_ms: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied { Opened, Updated, Dropped }

/// The bar rule for one (upstream, tf). Bucket start = ts - ts % span, ts in whole seconds
/// (ts_ms div 1000). Invariant: `cur.time` never decreases.
#[derive(Clone, Debug)]
pub struct BarBuilder { span: i64, late: LatePolicy, cur: Option<Bar> }

impl BarBuilder {
    /// `seed` is the last history bar; None (empty history) opens on the first tick, for every source.
    pub fn new(tf: Tf, late: LatePolicy, seed: Option<Bar>) -> BarBuilder { todo!() }
    pub fn apply(&mut self, t: &Tick) -> Applied {
        // TODO:
        // start = (t.ts_ms div 1000) - (t.ts_ms div 1000) % span
        // match cur {
        //   None                       => cur = open(start, t); Opened
        //   Some(c) if start > c.time  => cur = open(start, t); Opened
        //   Some(c) if start == c.time => update(c, t); Updated
        //   Some(c) /* start < c.time */ => match late { MergeIntoCurrent => update(c, t); Updated, Drop => Dropped }
        // }
        // update: high = max, low = min, close = price, volume += qty
        todo!()
    }
    pub fn current(&self) -> Option<&Bar> { self.cur.as_ref() }
}

/// At most one admitted update per `min_gap`; no trailing flush (Python parity).
#[derive(Clone, Debug)]
pub struct Throttle { min_gap: Option<Duration>, last: Option<Instant> }
impl Throttle {
    pub fn new(min_gap: Option<Duration>) -> Throttle { todo!() }
    /// True and records `now` when `now - last >= min_gap` (or there is no gap or no last).
    pub fn admit(&mut self, now: Instant) -> bool { todo!() }
}

// crates/hub-core/src/market/feed_health.rs (01-02)
/// `record_tick` + `health_report` (server.py:536-552) for one upstream. One writer: HubState::ingest.
#[derive(Clone, Debug, Default)]
pub struct FeedHealth { ticks: u64, last_tick_true_ms: Option<f64>, lags: VecDeque<f64> /* cap 300 */ }
impl FeedHealth {
    /// Returns the lag (true now minus src_ms) for the `bar` frame's `lag_ms`; None when src_ms is None.
    pub fn record(&mut self, now_true_ms: f64, src_ms: Option<i64>) -> Option<f64> { todo!() }
    /// median = sorted[len / 2], p95 = sorted[floor(len * 0.95)], age = now - last tick (0 base if none).
    pub fn entry(&self, now_true_ms: f64) -> FeedHealthEntry { todo!() }
    pub fn ticks(&self) -> u64 { self.ticks }
    pub fn lags(&self) -> impl Iterator<Item = f64> + '_ { self.lags.iter().copied() }
}

// crates/hub-core/src/market/backoff.rs (01-02)
/// Exponential backoff, 1 s to 60 s, full jitter, for every provider. Pure: the caller passes a
/// uniform number in [0, 1), so tests are deterministic.
#[derive(Clone, Debug)]
pub struct Backoff { base: Duration, cap: Duration, attempt: u32 }
impl Backoff {
    pub fn new(base: Duration, cap: Duration) -> Backoff { todo!() }
    /// delay = unit * min(cap, base * 2^attempt); attempt += 1 (saturating).
    pub fn next(&mut self, unit: f64) -> Duration { todo!() }
    /// Called when a session delivered data.
    pub fn reset(&mut self) { self.attempt = 0 }
}
```

### hub-core: sources (REST half 01-01, live half 01-02)

```rust
// crates/hub-core/src/sources/mod.rs
/// Production URLs by default; tests point them at local fakes (ws_e2e.rs, bars_cache tests).
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub binance_rest: String,   // https://api.binance.com
    pub binance_ws: String,     // wss://data-stream.binance.vision/ws/ (443; 9443 is blocked)
    pub bybit_rest: String,     // https://api.bybit.com
    pub bybit_ws: String,       // wss://stream.bybit.com/v5/public/
    pub deriv_ws: String,       // wss://api.derivws.com/trading/v1/options/ws/public
}
impl Default for Endpoints { fn default() -> Self { todo!() } }

/// The provider clients: one reqwest client (rustls, 15 s timeout) and the Deriv RPC pool.
#[derive(Clone)]
pub struct Sources { http: reqwest::Client, deriv: deriv::RpcPool, ep: Arc<Endpoints> }

impl Sources {
    pub fn new(ep: Endpoints) -> Sources { todo!() }
    pub fn endpoints(&self) -> &Endpoints { &self.ep }
    /// Dispatch by source. Oldest first. The provider caps are inside: Binance and Bybit at most 1000
    /// (no pagination), Deriv any limit through parallel windows.
    pub async fn history(&self, q: &BarsQuery) -> Result<Vec<Bar>, FeedError> { todo!() }
    pub async fn deriv_symbols(&self) -> Result<Vec<Symbol>, FeedError> { todo!() }
    pub async fn bybit_symbols(&self) -> Result<Vec<Symbol>, FeedError> { todo!() }
}

/// Display text is what the browser sees inside "<Source> history failed: <e>" and the 502 body.
#[derive(Debug, Clone, thiserror::Error)]
pub enum FeedError {
    #[error("HTTP Error {status}: {reason}")] Status { status: u16, reason: String },
    #[error("{0}")] Transport(String),
    #[error("{0}")] Upstream(String),     // Bybit retMsg ("Bybit: <msg>"), Deriv error.message
    #[error("bad upstream data: {0}")] Decode(String),
    #[error("timed out")] Timeout,
}

// ---- 01-02 additions to sources/mod.rs ----

/// One upstream's identity. Binance is keyed by the mapped symbol, so BTCUSD and BTCUSDT share one
/// `@aggTrade` stream; Bybit and Deriv by the requested ticker (BYBIT_X and BYBIT_X.MARK differ).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UpstreamKey { pub source: Source, pub ident: Ticker }
impl UpstreamKey {
    pub fn new(source: Source, requested: &Ticker) -> UpstreamKey { todo!() }
    /// "<source>:<ident>" for /api/health.
    pub fn health_key(&self) -> String { todo!() }
    /// The `symbol` field of the `history` message: mapped for Binance, echo for the others.
    pub fn history_symbol(&self, requested: &Ticker) -> String { todo!() }
}

/// Everything the shell loop needs to run one upstream, as data.
#[derive(Clone, Debug)]
pub struct LivePlan {
    pub url: String,
    /// Frames sent after connect (Bybit subscribe op, Deriv ticks subscribe). Binance: none.
    pub hello: Vec<String>,
    pub keepalive: KeepAlive,
    pub parser: FrameParser,
    pub rules: FeedRules,
    /// "Binance" | "Bybit" | "Deriv", for "<label> reconnecting: <e>".
    pub label: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub enum KeepAlive {
    /// Protocol ping frame (Binance, Deriv; Python used the library default of 20 s).
    WsPing(Duration),
    /// Bybit: library ping off, JSON {"op":"ping"} every 20 s, required by Bybit.
    AppPing { every: Duration, text: &'static str },
}

/// Dispatches to the provider's pure frame parser.
#[derive(Clone, Debug)]
pub enum FrameParser {
    Binance,
    Bybit { topic: String, field: Option<&'static str> },  // field: markPrice | indexPrice
    Deriv,
}
impl FrameParser { pub fn parse(&self, text: &str) -> Parsed { todo!() } }

#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    Batch(TickBatch),
    /// Deriv error reply: broadcast `status` "Deriv: <msg>", end the session, back off.
    EndSession { status: String },
    /// Pongs, subscribe acks, other topics, empty mark/index values.
    Ignore,
}

pub fn live_plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan { todo!() }

// crates/hub-core/src/sources/binance.rs
/// XAUUSD -> PAXGUSDT; any other XXXUSD -> XXXUSDT; else unchanged (server.py:53).
pub fn bsym_of(symbol: &str) -> String { todo!() }
/// The 9 fixed entries of server.py:716, in order.
pub fn symbols() -> Vec<Symbol> { todo!() }
pub(crate) async fn history(http: &reqwest::Client, ep: &Endpoints, q: &BarsQuery) -> Result<Vec<Bar>, FeedError> { todo!() }
mod wire { /* private serde DTOs: kline rows as [Value; 12], AggTrade { p: String, q: String, T: i64 } */ }
pub(crate) fn parse_klines(body: &[u8]) -> Result<Vec<Bar>, FeedError> { todo!() }
// 01-02:
pub(crate) fn rules() -> FeedRules { todo!() }                // MergeIntoCurrent, Some(50 ms), trade_ms
pub(crate) fn plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan { todo!() }  // <bsym lower>@aggTrade
pub(crate) fn parse_frame(text: &str) -> Parsed { todo!() }    // one Tick, src_ms = T

// crates/hub-core/src/sources/bybit.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Category { Linear, Spot, Inverse }
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum PriceKind { Trade, Mark, Index }
/// BYBIT_S_BTCUSDT.MARK -> (Spot, "BTCUSDT", Mark). Pure port of bybit_parse (server.py:375).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BybitTicker { pub cat: Category, pub sym: String, pub kind: PriceKind }
impl BybitTicker { pub fn parse(ticker: &str) -> BybitTicker { todo!() } }
/// instruments-info pages (cursor) + tickers turnover, status == "Trading" only, mark and index
/// rows for non-spot, sorted by group order then turnover desc. Transform is pure (`rows_to_symbols`).
pub(crate) async fn symbols(http: &reqwest::Client, ep: &Endpoints) -> Result<Vec<Symbol>, FeedError> { todo!() }
pub(crate) fn rows_to_symbols(per_cat: Vec<(Category, Vec<wire::Instrument>, HashMap<String, f64>)>) -> Vec<Symbol> { todo!() }
/// kline | mark-price-kline | index-price-kline, newest first upstream, reversed here.
pub(crate) async fn history(http: &reqwest::Client, ep: &Endpoints, q: &BarsQuery) -> Result<Vec<Bar>, FeedError> { todo!() }
mod wire { /* Envelope { retCode, retMsg, result }, Instrument, Ticker, Kline rows */ }
// 01-02:
pub(crate) fn rules() -> FeedRules { todo!() }                // Drop, Some(50 ms), trade_ms
pub(crate) fn plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan { todo!() }  // publicTrade.<SYM> or tickers.<SYM>
/// Trade topic: every trade in `data`, src_ms = last T. Tickers: one Tick of the mark/index value
/// with qty 0, src_ms = ts; an absent or empty value is Ignore.
pub(crate) fn parse_frame(text: &str, topic: &str, field: Option<&str>) -> Parsed { todo!() }

// crates/hub-core/src/sources/deriv.rs
/// Pooled request/response sockets (max 6 idle), req_id matching, one retry on a fresh socket
/// (server.py:565-592). The only async state machine on the REST side.
#[derive(Clone)]
pub struct RpcPool { inner: Arc<PoolInner> }
struct PoolInner { url: String, idle: Mutex<Vec<DerivConn>>, next_id: AtomicU64 }
impl RpcPool {
    pub fn new(url: String) -> RpcPool { todo!() }
    /// Sends `req` + req_id, reads until the reply with that req_id (or none), 20 s per read.
    pub async fn request(&self, req: serde_json::Value) -> Result<serde_json::Value, FeedError> { todo!() }
}

/// Pure planner for deep history (limit > 1000): windows of 1000 * g seconds stepping back from
/// `end`, up to 10 per batch, at most 5 in flight (the shell's buffer_unordered), stop at `limit`
/// bars or 4 empty batches in a row (server.py:635-648).
pub struct WindowPlan { g: i64, end: i64, limit: usize, empty_batches: u8 }
impl WindowPlan {
    pub fn new(tf: Tf, to_ms: Option<i64>, now_s: i64, limit: usize) -> WindowPlan { todo!() }
    /// The next batch of (start, end) windows, or None when done.
    pub fn next_batch(&mut self, have: usize) -> Option<Vec<(i64, i64)>> { todo!() }
    /// Tell the planner how many new distinct bars the batch added.
    pub fn record(&mut self, added: usize) { todo!() }
}
pub(crate) async fn symbols(pool: &RpcPool) -> Result<Vec<Symbol>, FeedError> { todo!() }
pub(crate) fn active_symbols_to_symbols(reply: &serde_json::Value) -> Vec<Symbol> { todo!() } // pure
pub(crate) async fn history(pool: &RpcPool, q: &BarsQuery, now_s: i64) -> Result<Vec<Bar>, FeedError> { todo!() }
// 01-02:
pub(crate) fn rules() -> FeedRules { todo!() }                // MergeIntoCurrent, None, no trade_ms
pub(crate) fn plan(key: &UpstreamKey, ep: &Endpoints) -> LivePlan { todo!() }  // {"ticks": sym, "subscribe": 1}
/// `error` -> EndSession("Deriv: <message>"); `tick` -> one Tick (qty 0, ts epoch*1000), src_ms = epoch*1000.
pub(crate) fn parse_frame(text: &str) -> Parsed { todo!() }
```

### hub-core: catalog and cache (01-01)

```rust
// crates/hub-core/src/catalog.rs
/// Symbol lists per provider. Each list has exactly one writer (its refresh loop), so each sits in
/// its own `watch` and the merge happens at the read (separate-before-serializing-shared-state).
#[derive(Clone)]
pub struct Catalog {
    deriv: Arc<watch::Sender<SymbolList>>,
    bybit: Arc<watch::Sender<SymbolList>>,
    /// Keyed by (deriv.version, bybit.version), which fixes Python's "same total count" cache bug.
    instruments: Arc<Mutex<Option<((u64, u64), Arc<InstrumentsResponse>)>>>,
}

#[derive(Clone, Debug, Default)]
pub struct SymbolList { pub items: Arc<[Symbol]>, pub version: u64 }

impl Catalog {
    pub fn new() -> Catalog { todo!() }
    pub fn symbols(&self, ws_port: u16) -> SymbolsResponse { todo!() }
    /// O(1) derivation: deriv ok = list non-empty, msg "<n> symbols"; bybit "<n> charts";
    /// binance always {ok: true, msg: "public feed"}. No 5 s cache needed.
    pub fn status(&self) -> StatusResponse { todo!() }
    pub fn instruments(&self) -> Arc<InstrumentsResponse> { todo!() }
    /// Deriv: retry every 60 s until the first success, then stop (Python parity, see open questions).
    /// Bybit: 3,600 s after success, 60 s after failure. Logs "Deriv: <n> symbols loaded",
    /// "Bybit: <n> charts loaded", "Deriv symbols failed: <e>" with target "server".
    pub fn spawn_refresh(&self, sources: Sources) -> [JoinHandle<()>; 2] { todo!() }
}

// crates/hub-core/src/bars_cache.rs
/// The one fetch seam BarsCache needs; `Sources` implements it, tests implement it with a counter.
pub trait HistoryFetch: Clone + Send + Sync + 'static {
    fn history(&self, q: &BarsQuery) -> impl Future<Output = Result<Vec<Bar>, FeedError>> + Send;
}
impl HistoryFetch for Sources { /* forwards to Sources::history */ }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// Any slot younger than the query's TTL.
    Cached,
    /// A fetch that starts after this call (or one already in flight that did); used to seed a new
    /// builder so its forming bar is current.
    Fresh,
}

/// Monotonic totals per provider; readers diff. bars counts every bar handed out (HTTP and ws
/// history alike: feedlog's bars_served is their sum anyway).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ServeStats { pub bars: u64, pub errors: u64 }

/// LRU of 512 slots, single-flight per key, TTL by `BarsQuery::ttl`. Errors are never cached.
#[derive(Clone)]
pub struct BarsCache<F: HistoryFetch = Sources> { inner: Arc<CacheInner<F>> }
struct CacheInner<F> {
    fetch: F,
    slots: std::sync::Mutex<lru::LruCache<BarsQuery, Slot>>,
    stats: std::sync::Mutex<PerSource<ServeStats>>,
}
struct Slot { created: Instant, cell: Arc<tokio::sync::OnceCell<(Instant, Arc<Vec<Bar>>)>> }

impl<F: HistoryFetch> BarsCache<F> {
    pub fn new(fetch: F) -> Self { todo!() }
    pub async fn get(&self, q: &BarsQuery, fresh: Freshness) -> Result<Arc<Vec<Bar>>, FeedError> {
        // TODO:
        // lock slots: take the slot for q; replace it with a new empty cell when it is filled and
        //   (age > q.ttl() or (fresh == Fresh and filled before this call)); unlock.
        // cell.get_or_try_init(|| fetch.history(q)) -- concurrent callers share one fetch.
        // Ok: stats[q.source].bars += n. Err: stats[q.source].errors += 1, remove the slot if it is
        //   still this cell, tracing::error!(target: "feed.<src>", every = 30,
        //   "{symbol} {tf}: bars request failed: {e}").
        todo!()
    }
    pub fn stats(&self) -> PerSource<ServeStats> { todo!() }
}
```

### hub-core: clock (01-02)

```rust
// crates/hub-core/src/clock.rs
/// Offset of the local clock against time.windows.com, measured once in the background at start
/// (the HTTP port binds first, as in Python). Stored as f64 bits in an atomic; 0 until measured.
#[derive(Clone, Default)]
pub struct Clock { offset_bits: Arc<AtomicU64> }

/// Everything the pure core needs to know about time, read once per event by the shell.
#[derive(Clone, Copy, Debug)]
pub struct Now { pub mono: Instant, pub local_ms: i64, pub true_ms: f64 }

impl Clock {
    pub fn now(&self) -> Now { todo!() }
    pub fn offset_ms(&self) -> f64 { todo!() }
    /// SNTP to time.windows.com:123, 5 s timeout. Logs (target "server")
    /// "Clock offset vs time.windows.com: <+N> ms (corrected in lag numbers)" or "clock offset failed: <e>".
    pub async fn measure(&self, server: &str) -> f64 { todo!() }
}
/// Pure: offset = server_time - (t1 + t4) / 2, from the 48-byte reply's transmit timestamp.
pub fn sntp_offset(t1_s: f64, t4_s: f64, reply: &[u8; 48]) -> f64 { todo!() }
```

### hub-core: feeds (01-02)

```rust
// crates/hub-core/src/feeds/state.rs
//! The pure registry. No I/O, no clock reads, no await. Every method takes `now`.
//! Holding a broadcast::Sender is fine here: send is synchronous and needs no runtime.

/// The serialized frame fanned out to viewers. Cheap to clone (Bytes inside).
pub type Frame = axum::extract::ws::Utf8Bytes;

/// Distinguishes an upstream entry from a later one with the same key, so a late call from a
/// stopped task or a stale guard is a no-op.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Epoch(u64);

pub struct Limits { pub max_upstreams: usize, pub per_ip: u16, pub grace: Duration, pub fanout: usize /* 64 */ }

pub struct HubState {
    limits: Limits,
    upstreams: HashMap<UpstreamKey, Upstream>,
    sockets_per_ip: HashMap<IpAddr, u16>,
    next_epoch: u64,
    totals: PerSource<FeedTotals>,
}

struct Upstream {
    epoch: Epoch,
    rules: FeedRules,
    /// Invariant: empty iff `idle_since` is Some. Only `add_viewer` and `remove_viewer` touch either.
    series: BTreeMap<Tf, Series>,
    idle_since: Option<Instant>,
    health: FeedHealth,
    /// Dropping the entry drops this sender, which ends the upstream task. The entry's existence is
    /// the upstream's life.
    _stop: oneshot::Sender<()>,
}

struct Series {
    /// None until the first joiner seeds it; ticks before that are ignored (no double counting).
    builder: Option<BarBuilder>,
    throttle: Throttle,
    viewers: u32,
    tx: broadcast::Sender<Frame>,
    last_data: Instant,
}

/// Monotonic, one writer (HubState). Feedlog diffs consecutive views.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FeedTotals { pub ticks: u64, pub updates: u64, pub reconnects: u64, pub errors: u64 }

/// What the shell must start after unlocking: spawn `upstream::run(hub, start)`.
pub struct UpstreamStart { pub key: UpstreamKey, pub epoch: Epoch, pub stop: oneshot::Receiver<()> }

pub struct Joined {
    pub epoch: Epoch,
    pub rx: broadcast::Receiver<Frame>,
    pub start: Option<UpstreamStart>,
    /// True when this join created the series: the shell logs "<SYM TF>: chart stream opened".
    pub series_opened: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Feed capacity reached ({max} symbols); try again later")]
pub struct AtCapacity { pub max: usize }

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("too many sockets from this address ({max})")]
pub struct PerIpLimit { pub max: u16 }

/// An emit to serialize and send after unlocking. Built under the lock, sent outside it.
pub struct Emit { pub tx: broadcast::Sender<Frame>, pub msg: ServerMsg }

impl HubState {
    pub fn new(limits: Limits) -> HubState { todo!() }

    pub fn admit_socket(&mut self, ip: IpAddr) -> Result<(), PerIpLimit> { todo!() }
    /// Removes the map entry at 0 so the map does not grow with every address ever seen.
    pub fn release_socket(&mut self, ip: IpAddr) { todo!() }

    pub fn is_seeded(&self, key: &UpstreamKey, tf: Tf) -> bool { todo!() }

    /// Atomic: capacity, upstream creation, series creation, seeding, receiver, viewer count.
    pub fn join(&mut self, key: &UpstreamKey, tf: Tf, history: &[Bar], now: Instant) -> Result<Joined, AtCapacity> {
        // TODO:
        // if !upstreams.contains(key):
        //     if upstreams.len() >= max_upstreams:
        //         evict the idle upstream with the smallest idle_since, else Err(AtCapacity)
        //     create Upstream { epoch: next, rules: sources::rules(key.source), stop pair, .. }
        //     start = Some(UpstreamStart { .. })
        // series = upstream.series.entry(tf).or_insert_with(new series); series_opened
        // if series.builder.is_none(): builder = BarBuilder::new(tf, rules.late, history.last())
        // rx = series.tx.subscribe(); add_viewer (clears idle_since)
        todo!()
    }

    /// Idempotent per guard; a stale epoch is a no-op. Series at 0 viewers is removed (the shell
    /// logs "chart stream closed"); an upstream with no series gets idle_since = now.
    pub fn leave(&mut self, key: &UpstreamKey, epoch: Epoch, tf: Tf, now: Instant) -> LeaveOutcome { todo!() }

    /// The hot path. One health sample per batch; every series applies every tick; throttled emits.
    pub fn ingest(&mut self, key: &UpstreamKey, epoch: Epoch, batch: &TickBatch, now: Now) -> Vec<Emit> {
        // TODO:
        // up = upstreams.get_mut(key) filtered by epoch, else return []
        // lag = up.health.record(now.true_ms, batch.src_ms); totals[src].ticks += 1
        // for series in up.series.values_mut():
        //     builder = match &mut series.builder { Some(b) => b, None => continue }
        //     for t in batch.ticks { builder.apply(t); }
        //     if let Some(bar) = builder.current() and series.throttle.admit(now.mono):
        //         series.last_data = now.mono; totals[src].updates += 1
        //         push Emit { tx: series.tx.clone(), msg: ServerMsg::Bar { bar: *bar, sent: now.local_ms,
        //             trade_ms: rules.sends_trade_ms.then_some(batch.src_ms).flatten(), lag_ms: lag } }
        todo!()
    }

    /// A session ended or reconnected: one `status` to every series of the upstream, counted once.
    pub fn upstream_status(&mut self, key: &UpstreamKey, epoch: Epoch, msg: String) -> Vec<Emit> { todo!() }

    /// Counted once per refusal: history failure or capacity.
    pub fn count_error(&mut self, source: Source) { todo!() }
    pub fn count_history(&mut self, source: Source, bars: usize) { todo!() }

    /// Removes every upstream whose idle_since + grace <= now. Returns the removed keys for logging.
    pub fn sweep(&mut self, now: Instant) -> Vec<UpstreamKey> { todo!() }

    /// A copy for /api/health and feedlog. Lags are copied only when asked (feedlog's 60 s summary).
    pub fn view(&self, now: Now, clock_offset_ms: f64, detail: ViewDetail) -> FeedView { todo!() }
}

pub enum LeaveOutcome { Nothing, SeriesClosed, UpstreamIdle }
#[derive(Clone, Copy)] pub enum ViewDetail { Brief, WithLags }

#[derive(Clone, Debug)]
pub struct FeedView {
    pub true_ms: f64,
    pub clock_offset_ms: f64,
    pub upstreams: Vec<UpstreamView>,
    pub totals: PerSource<FeedTotals>,
}
#[derive(Clone, Debug)]
pub struct UpstreamView {
    pub key: UpstreamKey,
    pub health: FeedHealthEntry,
    pub lags: Vec<f64>,                 // empty unless WithLags
    pub idle: bool,
    pub series: Vec<SeriesView>,
}
#[derive(Clone, Debug)]
pub struct SeriesView { pub tf: Tf, pub viewers: u32, pub since_data: Duration }

impl FeedView {
    /// Pure: /api/health from the view.
    pub fn health(&self) -> HealthResponse { todo!() }
}

// crates/hub-core/src/feeds/hub.rs
/// The shell around HubState: one std Mutex, never held across await.
#[derive(Clone)]
pub struct FeedHub { inner: Arc<Shared> }
struct Shared { state: std::sync::Mutex<HubState>, bars: BarsCache, sources: Sources, clock: Clock }

/// What the /ws route asked for, parsed at the boundary (routes/ws.rs::parse_subscribe).
#[derive(Clone, Debug)]
pub struct Wanted { pub key: UpstreamKey, pub tf: Tf, pub requested: Ticker }

impl FeedHub {
    pub fn new(cfg: &Config, bars: BarsCache, sources: Sources, clock: Clock) -> FeedHub { todo!() }

    pub fn admit_socket(&self, ip: IpAddr) -> Result<SocketPermit, PerIpLimit> { todo!() }

    /// The whole subscribe: peek seeded, fetch history through the cache (Fresh when unseeded),
    /// join under the lock, spawn the upstream if the join asked for it, log "chart stream opened".
    pub async fn subscribe(&self, want: Wanted) -> Result<Subscription, Refusal> { todo!() }

    pub fn health(&self) -> HealthResponse { todo!() }
    pub fn view(&self, detail: ViewDetail) -> FeedView { todo!() }

    /// Every 1 s: state.sweep(now); logs "<SYM>: upstream closed after grace" per removed key.
    pub fn spawn_sweeper(&self) -> JoinHandle<()> { todo!() }

    // Used by feeds/upstream.rs only. Both lock, call the pure method, unlock, then serialize each
    // Emit once and `tx.send` it (a send error means no receivers, which is fine).
    pub(crate) fn ingest(&self, key: &UpstreamKey, epoch: Epoch, batch: &TickBatch) { todo!() }
    pub(crate) fn status(&self, key: &UpstreamKey, epoch: Epoch, msg: String) { todo!() }
    pub(crate) fn sources(&self) -> &Sources { &self.inner.sources }
}

/// RAII: dropping it releases the per-IP slot.
pub struct SocketPermit { hub: FeedHub, ip: IpAddr }
impl Drop for SocketPermit { fn drop(&mut self) { /* lock; release_socket(ip) */ } }

/// RAII: dropping it (socket closed, replaced by a new subscribe, task aborted) calls leave().
pub struct Subscription {
    hub: FeedHub,
    key: UpstreamKey,
    epoch: Epoch,
    tf: Tf,
    history: Frame,
    rx: broadcast::Receiver<Frame>,
}
impl Subscription {
    /// The `history` frame, built once at subscribe time.
    pub fn history(&self) -> Frame { self.history.clone() }
    /// Next live frame; `Lagged` is skipped (each bar frame is a full snapshot), `Closed` ends.
    pub async fn next(&mut self) -> Option<Frame> { todo!() }
}
impl Drop for Subscription { fn drop(&mut self) { /* lock; leave(key, epoch, tf, now); log closed */ } }

#[derive(Debug, Clone)]
pub enum Refusal {
    /// "<Source> history failed: <e>" (Python's text per source).
    History(String),
    Capacity(AtCapacity),
}
impl Refusal {
    /// `error` frame; Capacity carries `kind: "capacity"` (D-002), History has no kind.
    pub fn frame(&self) -> Frame { todo!() }
}

// crates/hub-core/src/feeds/upstream.rs
/// One task per upstream entry. Ends when the entry is removed (stop resolves) and never otherwise.
pub(crate) async fn run(hub: FeedHub, start: UpstreamStart) {
    // TODO:
    // let plan = sources::live_plan(&start.key, hub.sources().endpoints());
    // let mut backoff = Backoff::new(1 s, 60 s);
    // tracing::info!(target: feed_target(src), "{ident}: upstream opened");
    // loop {
    //   select! { _ = &mut stop => return, end = session(&hub, &start, &plan, &mut backoff) => match end {
    //       SessionEnd::Lost(e)          => hub.status(key, epoch, format!("{} reconnecting: {e}", plan.label)),
    //       SessionEnd::Told(status)     => hub.status(key, epoch, status),   // Deriv error reply
    //   }}
    //   select! { _ = &mut stop => return, _ = sleep(backoff.next(fastrand::f64())) => {} }
    // }
    todo!()
}

enum SessionEnd { Lost(String), Told(String) }

/// connect (10 s open timeout, 25 s for Bybit), send hello, then read with the keepalive timer:
/// Text -> plan.parser.parse -> Batch: backoff.reset(); hub.ingest(..) | EndSession -> Told | Ignore.
/// A WsPing keepalive sends a ping frame every period and treats two missed pongs as Lost.
async fn session(hub: &FeedHub, start: &UpstreamStart, plan: &LivePlan, backoff: &mut Backoff) -> SessionEnd { todo!() }
```

### hub-core: routes (01-01, 01-02, 01-03)

```rust
// crates/hub-core/src/routes/mod.rs (01-01)
pub(crate) mod market;   // 01-02: ws, health; 01-03: logs
/// JSON body with `Content-Type: application/json` and `Cache-Control: no-store`.
pub(crate) fn json_ok<T: Serialize>(v: &T) -> Response { todo!() }
pub(crate) fn api_error(code: StatusCode, msg: &str) -> Response { todo!() }  // ErrorBody
pub(crate) async fn not_found() -> Response { todo!() }                        // 404 {"error":"not found"}

// crates/hub-core/src/routes/market.rs (01-01)
pub(crate) fn routes() -> Router<AppState> {
    // /api/status, /api/symbols, /api/instruments, /api/bars (GET; HEAD follows GET)
    todo!()
}
async fn status(State(st): State<AppState>) -> Response { json_ok(&st.catalog.status()) }
async fn symbols(State(st): State<AppState>) -> Response { json_ok(&st.catalog.symbols(st.cfg.http_port)) }
async fn instruments(State(st): State<AppState>) -> Response { json_ok(&*st.catalog.instruments()) }
async fn bars(State(st): State<AppState>, Query(p): Query<HashMap<String, String>>) -> Response { todo!() }

// crates/hub-core/src/routes/ws.rs (01-02)
pub(crate) fn routes() -> Router<AppState> { todo!() }   // GET /ws
async fn upgrade(State(st): State<AppState>, ClientIp(ip): ClientIp, ws: WebSocketUpgrade) -> Response { todo!() }

/// One socket: a writer fed by an mpsc (capacity 64) so the subscription task never touches the
/// socket, the current subscription's AbortHandle, ping every 30 s, close after 60 s without a pong.
/// The SocketPermit lives exactly as long as this future.
async fn serve_socket(socket: WebSocket, st: AppState, permit: SocketPermit) { todo!() }

#[derive(Debug, PartialEq)]
pub(crate) enum SubscribeError {
    /// Not JSON or not an object: Python's handler raised and closed the socket.
    CloseSocket,
    /// `{"type":"error","msg":"Unknown timeframe X"}`, socket stays open.
    UnknownTf(String),
    /// `{"type":"error","msg":"Unknown symbol X"}`, socket stays open (new boundary check).
    BadSymbol(String),
}
impl SubscribeError { pub(crate) fn frame(&self) -> Frame { todo!() } }

/// Pure. Defaults: source "binance" (then the fallback rule), symbol "XAUUSD", tf "M1".
pub(crate) fn parse_subscribe(text: &str) -> Result<Wanted, SubscribeError> { todo!() }

// crates/hub-core/src/routes/health.rs (01-02)
pub(crate) fn routes() -> Router<AppState> { todo!() }   // GET /api/health -> st.feeds.health()

// crates/hub-core/src/routes/logs.rs (01-03)
pub(crate) fn routes() -> Router<AppState> { todo!() }   // GET /api/logs, POST /api/logs/journal
/// `since` digits else 0; `brief` by presence; limit 3,000; adds journal_raw: false.
async fn logs(State(st): State<AppState>, Query(p): Query<HashMap<String, String>>) -> Response { todo!() }
/// net::post_guard first (403, 415, 413 on Content-Length), then read at most 300,000 bytes,
/// then {"ok": true}. The body is ignored (Python's journal stub).
async fn journal(State(st): State<AppState>, headers: HeaderMap, body: Body) -> Response { todo!() }
```

### hub-core: net (client IP 01-02, the rest 01-03)

```rust
// crates/hub-core/src/net.rs
/// Pure. `CF-Connecting-IP` is trusted only when the TCP peer is loopback (the tunnel); a header
/// that does not parse as an IP is ignored. Otherwise the peer address.
pub fn client_ip(peer: SocketAddr, cf_connecting_ip: Option<&HeaderValue>) -> IpAddr { todo!() }

/// Extractor over ConnectInfo<SocketAddr> + headers. Used by /ws (per-IP limit) and the rate limiter.
pub struct ClientIp(pub IpAddr);
impl<S: Send + Sync> FromRequestParts<S> for ClientIp { /* uses client_ip */ }

// ---- 01-03 ----
/// tower_governor key extractor using the same client_ip rule (one source of truth).
#[derive(Clone)] pub(crate) struct ClientIpKey;
impl tower_governor::key_extractor::KeyExtractor for ClientIpKey { /* .. */ }

/// 20 req/s, burst 40 per client IP on /api/* only; 429 with Retry-After and an ErrorBody.
pub(crate) fn rate_limit() -> GovernorLayer</* ClientIpKey, .. */> { todo!() }
/// X-Content-Type-Options: nosniff, Referrer-Policy: same-origin on every response (CSP is Phase 4).
pub(crate) fn security_headers() -> tower::layer::util::Stack</* SetResponseHeader x2 */> { todo!() }

/// Pure POST rule (server.py:989-1013): an Origin, when present, must be one of
/// cfg.allowed_origins() (403 "forbidden origin"); Content-Type must contain application/json
/// (415 "JSON only"); Content-Length over 300,000 is 413 "too large".
pub(crate) fn post_guard(cfg: &Config, headers: &HeaderMap) -> Result<(), (StatusCode, &'static str)> { todo!() }
```

### hub-core: logbus and feedlog (01-03)

```rust
// crates/hub-core/src/logbus.rs
pub const RING_CAP: usize = 12_000;
pub const MSG_CAP: usize = 2_000;
pub const THROTTLE_TABLE_CAP: usize = 2_000;

/// One line to add, already split from a tracing event by RingLayer (or built by feedlog).
#[derive(Clone, Debug)]
pub struct Line {
    pub src: String,
    pub lvl: Level,
    pub msg: String,        // message plus " k=v" facts, unredacted
    pub every_s: u32,       // 0 = no throttle; drops the same (src, msg) within N s
    pub persist: bool,      // write to hub.log
    pub alive: bool,        // counts as the source writing (sources[src].last)
    pub t_ms: Option<i64>,
}

/// The pure ring (logbus.py's RING, SOURCES, PROBLEMS, _throttle, _seq).
pub struct LogRing {
    boot_s: i64,
    seq: u64,
    rows: VecDeque<LogRow>,
    throttle: HashMap<(String, String), f64>,
    sources: BTreeMap<String, SourceInfo>,
    problems: BTreeMap<String, String>,
}

impl LogRing {
    pub fn new(boot_s: i64) -> LogRing { todo!() }
    /// Redact, cap at 2,000 chars (on a char boundary), throttle, append with the next id, update
    /// sources[src].last when alive. Returns the row when it was added and `persist` is set, for
    /// the file sink. Wipes the throttle table at 2,000 entries, as Python.
    pub fn push(&mut self, now_ms: f64, line: Line) -> Option<LogRow> { todo!() }
    /// Boot reload: the tail of hub.log (last ~1 MB, first partial line dropped), last 4,000 lines
    /// that parse, fresh ids; sources[src].last from their times.
    pub fn reload(&mut self, file_tail: &str, limit: usize) { todo!() }
    /// Replace every problem whose key starts with `prefix` by `desired` (all of which must start
    /// with `prefix`). Idempotent; a stale problem cannot survive a step.
    pub fn replace_problems(&mut self, prefix: &str, desired: BTreeMap<String, String>) { todo!() }
    /// errors_15m and latest_error over rows with lvl error and now - t < 900,000.
    pub fn snapshot(&self, now_ms: f64, since: u64, limit: usize, brief: bool) -> LogSnapshot { todo!() }
}

/// The four logbus.py patterns, in order: bearer <x>; authorization|token|api[_-]?key|apikey|
/// secret|passw(or)?d|pwd followed by [:=] or whitespace and a value; 32+ hex; 40+ of
/// [A-Za-z0-9+_-] with optional "=" padding. `regex` supports every construct used.
pub fn redact(text: &str) -> Cow<'_, str> { todo!() }
/// "2026-11-23T10:00:00.123Z info  feed.binance <msg with newlines as spaces>"
pub fn format_file_line(row: &LogRow) -> String { todo!() }
pub fn parse_file_line(line: &str) -> Option<(i64, Level, String, String)> { todo!() }

/// The shell: the ring behind a std Mutex, a file sink, the clock.
#[derive(Clone)]
pub struct LogBus { ring: Arc<std::sync::Mutex<LogRing>>, file: FileSink, clock: Clock }
impl LogBus {
    /// Creates HUB_LOG_DIR, reloads the tail of hub.log, starts the writer thread.
    pub fn open(dir: &Path, clock: Clock) -> std::io::Result<LogBus> { todo!() }
    pub fn layer(&self) -> RingLayer { RingLayer { bus: self.clone() } }
    pub fn push(&self, line: Line) { todo!() }
    pub fn replace_problems(&self, prefix: &str, desired: BTreeMap<String, String>) { todo!() }
    pub fn snapshot(&self, since: u64, brief: bool) -> LogSnapshot { todo!() }
}

/// Bounded std channel to one writer thread (never blocks a tokio worker; a full channel drops the
/// file line, the ring keeps it). Rotation 2 MB x 3 (hub.log, hub.log.1 .. hub.log.3).
#[derive(Clone)]
struct FileSink { tx: std::sync::mpsc::SyncSender<String> }

/// tracing Layer: target -> src (third-party targets containing "::" only at warn and above),
/// level -> lvl, message + other fields -> msg, and the fields `every`, `persist`, `alive`.
pub struct RingLayer { bus: LogBus }
impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for RingLayer { /* on_event */ }

// crates/hub-core/src/feedlog.rs
pub const SUMMARY: Duration = Duration::from_secs(60);
pub const STREAM_SILENT: Duration = Duration::from_secs(120);

pub struct FeedLogInput<'a> {
    pub now: Instant,
    pub feeds: &'a FeedView,
    pub status: &'a StatusResponse,
    pub bars: &'a PerSource<ServeStats>,
}

pub struct FeedLogOutput {
    pub lines: Vec<Line>,
    /// The complete desired set of problems under "feed.": "feed.<k>" when a status light is down,
    /// "feed.<src>:<SYM TF>" for a series silent over 120 s.
    pub problems: BTreeMap<String, String>,
}

/// Pure feed-log state: previous totals (to diff the monotonic counters), the silent series set,
/// the previous status (for edge-triggered "up: <msg>" / "DOWN: <msg>" lines), last summary time.
pub struct FeedLog {
    last_summary: Instant,
    prev_totals: PerSource<FeedTotals>,
    prev_bars: PerSource<ServeStats>,
    silent: BTreeSet<(UpstreamKey, Tf)>,
    prev_status: Option<StatusResponse>,
}

impl FeedLog {
    pub fn new(now: Instant) -> FeedLog { todo!() }
    /// True when the next step will write the summary (so the caller asks for lags).
    pub fn wants_lags(&self, now: Instant) -> bool { todo!() }
    pub fn step(&mut self, input: FeedLogInput<'_>) -> FeedLogOutput {
        // TODO:
        // 1. status edges per k in deriv, bybit, binance: on change (or first step) a line on
        //    "feed.<k>", info "up: <msg>" or warn "DOWN: <msg>"; problem when !ok.
        // 2. silence per series: since_data > 120 s -> warn once "<SYM TF>: no data for N s (market
        //    closed, or the feed is dead)" + problem; recovery -> info "<SYM TF>: data flowing again".
        // 3. every 60 s, per provider that has series or nonzero deltas, one info line on "feed.<src>":
        //    "60 s: streams=N [SYM:TF,...] updates=U ticks/s=X.X lag_med=Ams lag_p95=Bms
        //     bars_served=H reconnects=R errors=E last_data=Ns" (lag=n/a, last_data=n/a when empty).
        //    streams counts series, not viewers; updates counts emitted frames, not frames per viewer;
        //    bars_served and errors add the BarsCache deltas.
        todo!()
    }
}

/// Every 2 s: view, step, push lines, replace_problems. Errors log "feed log: <e>" every=60.
pub fn spawn(st: AppState) -> JoinHandle<()> { todo!() }
```

### hub-server (01-03)

```rust
// crates/hub-server/src/main.rs
//! Thin: everything testable lives in hub-core.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // TODO:
    // 1. dotenvy: ./.env then hub/.env; real env wins, first file wins (setdefault semantics:
    //    dotenvy::from_path does not override, so load in that order).
    // 2. let cfg = Config::from_env()?;  let clock = Clock::default();
    // 3. let logs = LogBus::open(&cfg.log_dir, clock.clone())?;
    //    tracing_subscriber::registry().with(logs.layer()).with(fmt message-only to stdout).init();
    //    panic hook -> tracing::error!(target: "server.err", ..)
    //    tracing::info!(target: "server", "Chart server started (pid {})", std::process::id());
    // 4. let listener = TcpListener::bind(cfg.bind_addr()).await?   (port taken -> clear error)
    // 5. let state = AppState::build_parts(cfg, clock, logs).await?;  let bg = state.spawn_background();
    //    (background SNTP logs the "Clock offset vs time.windows.com: ..." line)
    // 6. tracing::info!(target: "server",
    //      "TradeX: open http://localhost:{port}/hub.html  (data on ws://localhost:{port}/ws)");
    // 7. hub_core::serve(listener, state) with ctrl_c graceful shutdown; bg.shutdown().
    todo!()
}
```

### Tests that prove the shape without a network (named, not written)

```rust
// market/candles.rs
#[test] fn binance_late_trade_merges_into_current() {}
#[test] fn bybit_late_trade_is_dropped() {}
#[test] fn deriv_volume_stays_zero() {}
#[test] fn replay_fixture_matches_python_bars() {}   // tests/fixtures/binance_btcusdt_aggtrade.jsonl
#[test] fn throttle_admits_one_per_50ms_and_has_no_trailing_flush() {}
// feeds/state.rs
#[test] fn second_viewer_same_symbol_other_tf_reuses_upstream() {}
#[test] fn capacity_counts_source_symbol_pairs_not_tfs() {}
#[test] fn capacity_evicts_longest_idle_before_refusing() {}
#[test] fn grace_keeps_upstream_until_60s_then_sweep_drops_stop_sender() {}
#[test] fn rejoin_within_grace_reuses_upstream_and_reseeds_series() {}
#[test] fn stale_epoch_leave_and_ingest_are_noops() {}
#[test] fn per_ip_limit_is_24_and_release_removes_entry() {}
#[test] fn ingest_counts_ticks_once_regardless_of_viewers() {}
#[test] fn ticks_before_seed_are_ignored() {}
// static_files.rs
#[test] fn probes_are_refused() {}    // %2e%2e/, %5c, .env, vendor/a/b.js, server.py, /.git/config
// feedlog.rs, logbus.rs
#[test] fn summary_line_format_matches_python() {}
#[test] fn silent_series_raises_and_clears_problem() {}
#[test] fn redaction_table_matches_logbus_py() {}
#[test] fn throttle_drops_same_src_msg_within_every() {}
// tests/ws_e2e.rs: fake Binance ws + fake REST on 127.0.0.1, two sockets, one upstream connection.
```

---

## Plan ownership

| Plan | Owns (types and modules) | Compiles and tests alone because |
|---|---|---|
| 01-01 | Workspace `Cargo.toml`, `rust-toolchain.toml`, `.gitignore`. `hub-types::market` (`Source`, `Tf`, `Bar`, `Symbol`, `SymbolGroup`, `Off`, `SymbolsResponse`, `FeedStatus`, `StatusResponse`, `InstrumentsResponse`, `Broker`, `Instrument`, `Variant`, `ErrorBody`). `hub-core`: `Config`, `AuthMode`, `ConfigError`; `AppState { cfg, catalog, bars }`, `app()`, `serve()`, `BackgroundTasks`; `static_files`; `market::{Ticker, PerSource, BarsQuery, instruments}`; `sources::{Sources, Endpoints, FeedError}` and the REST halves of `binance`, `bybit`, `deriv` (`bsym_of`, `BybitTicker`, `RpcPool`, `WindowPlan`, history, symbols); `Catalog`; `BarsCache`, `HistoryFetch`, `Freshness`, `ServeStats`; `routes::{mod, market}`; `tests/contract.rs` with the market, static and probe cases. | Nothing in it names a feed, a clock or the log bus. Logging is plain `tracing` to stdout. The contract suite serves `app()` in-process on `127.0.0.1:0` when `CONTRACT_RS_URL` is unset, so G1 for `/api/status`, `/api/symbols`, `/api/instruments`, `/api/bars` and the static probes runs before any binary exists. `BarsCache` single-flight and TTL are tested with a counting `HistoryFetch`. |
| 01-02 | `hub-types::{ws, health}` (`Subscribe`, `ServerMsg`, `ErrorKind`, `HealthResponse`, `FeedHealthEntry`). `hub-core`: `market::{candles, feed_health, backoff}` (`Tick`, `TickBatch`, `BarBuilder`, `LatePolicy`, `Throttle`, `FeedRules`, `FeedHealth`, `Backoff`); `clock` (`Clock`, `Now`, `sntp_offset`); the live halves of the three provider modules and `sources::{UpstreamKey, LivePlan, KeepAlive, FrameParser, Parsed}`; `feeds::{state, hub, upstream}` (`HubState`, `Epoch`, `Limits`, `FeedTotals`, `FeedView`, `Emit`, `FeedHub`, `Subscription`, `SocketPermit`, `Refusal`, `Wanted`); `net::{client_ip, ClientIp}`; `routes::{ws, health}`; `AppState` gains `clock`, `feeds`; fixtures; `tests/ws_e2e.rs`; contract cases for `/ws` and `/api/health`. | It depends only on 01-01 types. Feed events go through `tracing` with `feed.<src>` targets; no `LogBus` is named. `HubState` and every parser are tested with recorded frames and explicit `Instant`s. `ws_e2e.rs` uses `AppState::build_with(cfg, Endpoints { .. local fakes })`, so the full subscribe, fan-out, grace and capacity path runs with no internet. |
| 01-03 | `hub-types::logs` (`Level`, `LogRow`, `SourceInfo`, `LogSnapshot`, `LogsResponse`). `hub-core`: `logbus` (`LogRing`, `Line`, `redact`, `LogBus`, `RingLayer`, `FileSink`); `feedlog` (`FeedLog`, `FeedLogInput`, `FeedLogOutput`, `spawn`); `net::{ClientIpKey, rate_limit, security_headers, post_guard}`; `routes::logs`; `AppState` gains `logs`; `hub-server/src/main.rs`; `hub/lab.js wsUrl()` (`/ws` on the page origin, `wss:` on https); `hub/hub.html` `restartChart()` without the arm branch; `Procfile`, `scripts/dev.sh`, `.github/workflows/ci.yml`, `AGENTS.md`; contract cases for `/api/logs` and `POST /api/logs/journal`. | It reads only public views from 01-02 (`FeedHub::view`, `FeedView`) and 01-01 (`Catalog::status`, `BarsCache::stats`); `RingLayer` captures the `tracing` events those plans already emit, so no earlier file changes except `app()` (two layers, one merge) and `AppState` (one field). `LogRing` and `FeedLog::step` are pure and tested with synthetic views. CI points `CONTRACT_RS_URL` at the built binary. |
