# Learnings

<!--
Rules (paul-unify enforces these; keep them in this header):
- One row per lesson. Before adding a row, look for one with the same cause:
  a hit on it bumps `hits`, it is not a new row.
- `hits` counts independent occasions. A restatement of the same evidence by a
  later reflect pass is not a new occasion and does not bump it.
- status: candidate | validated | rejected
  candidate -> validated only at hits >= 3, or on a passing check, an explicit
  user confirmation, or a reviewer confirmation (rsi: a passing experiment).
  Never on reflection alone.
- applies-to: the loop stage where the rule acts: plan | apply | verify | unify | research.
  paul-plan turns every candidate into a Verification check at that stage
  (research rows only on research plans) and every validated row into a plan Fact.
- Ceiling: at most 30 active rows; archive the oldest below "## Archive".
- Expiry: prune a validated row not hit in 5 plans (move it to the archive).
- Experiments: a candidate that needs a before-and-after test gets an rsi run;
  its record lives at `.paul/experiments/<id>.md`.
-->

| id | status | rule | applies-to | hits | last-hit | evidence |
|----|--------|------|------------|------|----------|----------|
| L-001 | candidate | Measure elapsed time in async Rust code with `tokio::time::Instant`, never `std::time::Instant`, so paused-clock tests can drive TTLs and timers | apply | 2 | 02-02 | 01-01: BarsCache TTL test failed under `start_paused` until the cache switched to tokio Instant; 02-02: budget, futures list and store lifecycle tests run under start_paused with tokio Instant |
| L-002 | candidate | Before starting the Python oracle, check that ports 8770 and 8765 are free; if a `python hub/server.py` from this repo already holds them and `git diff main -- hub/` is empty, use it as `CONTRACT_PY_URL` and leave it running | apply | 1 | 01-01 | 01-01: started server exited (8765 in use) while PID 21944 from this repo served 8770 |
| L-003 | validated | Give every new contract or oracle check one mutation run (break one field, see FAIL, restore, see PASS) before trusting a green result | verify | 3 | 02-01 | 01-01: renaming `arm` produced `FAIL symbols: keys only in Python ["arm"]`, restore returned 18/18; 01-03: guard and key mutations each caught (and one sed mutation silently missed, see L-009); 02-01: three mutations each failed a named test; the run also showed the golden suite alone misses two of them |
| L-004 | candidate | Enable serde_json `float_roundtrip` before comparing floats with another implementation; the default fast parser can be off by one binary digit | verify | 1 | 01-02 | 01-02: fixture replay failed on `0.00045000000000000004` vs `0.00045` until the feature was on |
| L-005 | candidate | A fake that serves history must align its seed bar to the requested interval; a seed later than the bucket makes drop-late rules swallow every trade | verify | 1 | 01-02 | 01-02: ws_e2e saw no M5 bars because the fake seed was 1-minute aligned |
| L-006 | candidate | On Windows, measure work after wake-up separately from scheduled-arrival latency; the 15.6 ms timer tick dominates end-to-end numbers | verify | 1 | 01-02 | 01-02: replay_load p99 19 ms end to end vs 49.7 us processing |
| L-007 | candidate | After the gates pass, drive the real UI in a browser and read the live log; green tests do not prove runtime behaviour | verify | 1 | 01-03 | 01-03: 55 green tests, then Playwright showed "no data for 120 s" one second after an upstream opened |
| L-008 | candidate | Build test state with no file side effects; a relative default path resolves against the crate dir under cargo test | apply | 1 | 01-03 | 01-03: tests created crates/hub-core/hub/logs/hub.log with interleaved lines |
| L-009 | candidate | After applying a mutation, grep that it landed before reading a pass; rustfmt rewraps lines so sed can miss | verify | 2 | 02-01 | 01-03: the first "JSON only" mutation did not apply and the suite passed; re-applied by index, 1 FAIL; 02-01: every mutation was checked as applied before its run, and the restore was checked |
| L-010 | candidate | A scripted multi-file edit asserts each anchor occurs once and checks an already-applied marker before writing anything, formats only after all text edits, and re-reads the file before any re-run after a partial failure | apply | 1 | 01-03 | 01-03: an edit script half-applied, failed on an anchor rustfmt had rewrapped, and on re-run inserted the net.rs block twice |
| L-011 | candidate | In a parity port, take each stateful component's initial state and start-up order from the oracle's init path, one test per component, and compare a suspicious runtime number with the oracle's same measurement before fixing it | plan | 1 | 01-03 | 01-03: new upstream last_data None (false silence) and status check before catalog load (DOWN at boot) passed 55 tests and 25/0/0 contract; clock offset +45,529 ms matched Python's +45,524 ms |
| L-012 | candidate | Keep Playwright MCP state across run-code calls on the page object (page.on listeners filling page.__x), never on globalThis, and save screenshots under <repo>/.playwright-mcp | verify | 1 | 01-03 | 01-03: globalThis counters were undefined on the next call; a scratchpad screenshot path was refused |
| L-013 | candidate | After restoring a file from a backup, touch it; an older mtime makes cargo skip the rebuild and the next run tests a stale binary | apply | 1 | 02-01 | 02-01: a clean golden run reported 3,300 false differences from the binary still built with a mutation |
| L-014 | candidate | Binance aggTrades REST reaches only the last two days (code -4166 for startTime and fromId); build historical trade fixtures from the daily archive | plan | 1 | 02-01 | 02-01: pass 1 failed on HTTP 400 until segments were cut from the D+1 daily CSV |
| L-015 | candidate | A golden suite on rounded outputs can be blind to ulp-level rules; keep a table test per numeric rule and name which suite catches each mutation | verify | 2 | 02-02 | 02-01: the fold and sorted-fill mutations passed all 25 golden calls and failed only the unit tables; 02-02: the feed-key unit test passed its own mutation because the cache re-reads the last bar; it now asserts the request size |
| L-016 | candidate | Under tokio start_paused, keep sockets out of the test: the clock skips ahead while a task waits on I/O, so inject an in-memory source instead of a local fake server | apply | 1 | 02-02 | 02-02: the store lifecycle test uses an in-memory TradeSource; the one HTTP fake test runs on real time |
| L-017 | candidate | Before claiming a JSON comparator checks key order, confirm the parser keeps it (serde_json needs preserve_order) and mutate a field order to see it fail | verify | 1 | 02-03 | 02-03: the golden and replay suites compared sorted keys for two plans; a route test showed it |
| L-018 | candidate | Two hubs on one IP share Binance's 2,400 weight per minute; when a 429 appears in a side-by-side run, read X-MBX-USED-WEIGHT-1M with one hub stopped before blaming the code | verify | 1 | 02-03 | 02-03: the Python hub's own trade store back-fill caused the 429; the weight fell to 1 when it went idle |

## Archive

| id | status | rule | applies-to | hits | last-hit | evidence |
|----|--------|------|------------|------|----------|----------|
