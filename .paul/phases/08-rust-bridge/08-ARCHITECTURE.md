---
description: "vtradingview phase 08: the Rust MT5 bridge crate, its module map, parity contract and distribution"
type: Architecture
about: "vtradingview"
---

# Phase 08 Architecture: the Rust MT5 bridge

**Status:** contract for plans 08-01..08-03 (approved plan time 2026-11-26 unless amended)
**Oracle:** `bridge/mt5_bridge.py` (321 lines, measured this session)
**Base design:** candidate B (arena, single candidate — the runner for candidate A was stopped by
the owner before completing; recorded deviation from the two-candidate rule). The shape below is
candidate B's as synthesized.

## Problem

The MT5 bridge is a 321-line Python service (stdlib-only) running on a Windows host next to the
MT5 terminal. It becomes a single release binary distributed by mise, without changing one byte of
the 4-route JSON surface the hub parses (D-032), under the workspace's pinned deps (D-005), with
a parity oracle that proves equivalence rather than hoping for it. Deployment stays terminal-local:
the binary keeps speaking MCP (JSON-RPC 2.0 over HTTP) to the terminal's built-in MCP server.

## Shape (candidate B, synthesized)

```text
crates/mt5-bridge/            root workspace member (pure Rust, no GUI: cargo test --workspace
  Cargo.toml                    stays tauri-free; D-030's constraint does not apply)
  src/main.rs     thin shell: env + bridge.env (exe-relative, dotenvy no-overwrite), offset file
                  load, spawn offset task, router, serve. Fails fast: no BRIDGE_TOKEN = exit 1.
  src/mcp.rs      SEALED LEAF (no sibling imports, enforced in review): JSON-RPC 2.0 over reqwest.
                  Mcp { reqwest::Client, url, token, Mutex<Session{ id: Option<String>, next: u64 }> }
                  call(&self, name: &str, args: Value) -> Result<Value, McpError>
                  initialize (protocolVersion "2025-06-18", clientInfo "chart-bridge") +
                  notifications/initialized; session id from Mcp-Session-Id response header,
                  resent on every later call; response body scanned for `data:` lines (SSE-shaped
                  answers parse either way); content[].text joined and JSON-parsed; isError surfaces
                  as the error text; transport/parse failure reopens ONCE and retries (Python parity).
  src/term.rs     domain, parse-at-boundary: Tf(u32) with TryFrom<&str> M1..D1 (reject others);
                  Bar { time: i64, open/high/low/close/volume: f64 } (serde, Python key order);
                  bar_key = "time,close,high,low,volume"; Terminals { live, hist, misc: Arc<Mcp> }
                  (the Python LIVE/HIST/MISC split is an invariant: a long history read must never
                  delay the live candle); SyncPolicy { cap: Duration = 6 s, step: 400 ms } test-shrinkable;
                  bars() resync loop (span = limit*sec*1.5 + 4 days, x3 while short and under cap,
                  stale-vs-tick exit, client-side trim to limit, clamp 5000); last_tick (ms from
                  time_ms chars 20..23, absent = 0); symbols() (heartbeat category filtered,
                  category -> path map); account_health() (balance without equity = not logged in);
                  offset_measure() (server-minus-UTC rounded to 30 min, refuse if |d-h| >= 90 s).
  src/live.rs     tokio concurrency: PollerSet over watch channels.
                  Entry { tx: watch::Sender<LiveSnapshot>, asked: AtomicI64 };
                  LiveSnapshot { bar: Option<Bar>, key: String, tick_ms: Option<i64>, error: Option<String> };
                  get-or-spawn per (symbol, tf), poll every 250 ms on the LIVE session, idle reaper
                  at 60 s; wait(sym, tf, after, 20 s deadline) = read-or-await watch.
  src/http.rs     the only axum module: Bearer extractor (constant-time compare via `subtle`,
                  rejection 401 {"error":"unauthorized"}), the 4 GET routes, 404
                  {"error":"not found"}, errors as 502 {"error": str}; wire JSON built only here.
```

**Offsets are single-writer, many-reader:** the offset task owns `Offset`, readers hold a
`watch::Receiver<Offset>`; the poller set subscribes for offset changes. `/health` reports the
live `Offset` fields (`offset_s`, `offset_src`, `measured_at`).

**Interface depth:** the crate's public surface is `fn main()`; each module hides one capability
behind one call (`call`, `bars`, `wait`, `router`). JSON-RPC envelopes and SSE lines never leave
mcp.rs; MCP content strings parse into `Bar` in term.rs; wire shapes are produced only in http.rs.
Every data-flow trace stays three files deep (http -> term -> mcp).

## Parity contract (the D-032 frozen surface)

Byte-level rules the plans assert:

1. Routes and query names: `/health`, `/symbols`, `/bars?symbol=&tf=&limit=&to=`, `/wait?symbol=&tf=&after=`.
2. Auth: `Authorization: Bearer <BRIDGE_TOKEN>`, constant-time compare, else 401 `{"error":"unauthorized"}`.
3. `/bars` returns a **bare JSON array** of `{time, open, high, low, close, volume}` (time = UTC
   seconds offset-corrected; volume = float). Errors: 502 `{"error": "<text>"}`.
4. `/wait` returns `{"bar": <bar|null>, "key": "<time,close,high,low,volume>", "tick_ms": <int|null>}`;
   long-polls up to 20 s.
5. `/health` returns `{"ok": true, "server": <name>, "pollers": <n>, "offset_s": <int>,
   "offset_src": <str>, "measured_at": <int>}` (key order preserved).
6. `/symbols` returns an array of `{name, desc, path, digits}` with the same category-to-path map,
   heartbeat rows dropped.
7. Unknown path: 404 `{"error":"not found"}`. Unknown timeframe: 502 carrying Python's exact text
   (`unknown timeframe M2`).

The contract differ normalizes only the volatile scalars (`pollers`, `measured_at`) and otherwise
diffs canonical JSON (raw bytes for `/bars`).

## Test architecture

- **In-crate (`cargo test -p mt5-bridge`)**: a hand-rolled tokio TcpListener fake per concern;
  budgets shrunk through `SyncPolicy` (no test sleeps over 1 s; L-001 tokio Instant where a TTL
  matters).
- **Contract suite** `.paul/runs/check-mt5c.sh` (check-0703.sh pattern, self-terminating per
  L-040): starts (1) `python -I .paul/runs/fake_mcp_terminal.py` — a NEW stdlib fake that speaks
  MCP (POST, initialize, Mcp-Session-Id, tools/call; deterministic grid bars; a heartbeat symbol;
  a magic `ERRX` symbol that answers isError; a control file flipping logged-in state and plain-JSON
  vs SSE framing between passes; `/wait` derived from `after` alone so both bridges see identical
  keys) — then (2) the Rust bridge and (3) `python -I bridge/mt5_bridge.py`, same `BRIDGE_TOKEN`,
  each pointing at the one fake. The differ `.paul/runs/diff_bridge_contract.py` walks all four
  routes (all TFs, limit and `to` variants, `/wait` with empty and current-key `after`) on both
  and diffs. The fake being Python is deliberate: an independent implementation of MCP framing
  cannot collude with a bug in the Rust client.
- **Mutations** (each at a guard a named check observes; L-003/L-009 landing markers, re-run after
  any checker edit per R-10, `--list` count asserted per L-026):
  offset-round, session-reopen, key-fields (volume dropped from bar_key), limit-clamp, auth-exact
  (prefix-match relaxation caught by a near-miss token), wait-deadline.

## Distribution

- `release.yml` strategy additions: `x86_64-pc-windows-msvc` -> `mt5-bridge-vX.Y.Z-windows-x64.zip`
  (windows-latest, exe at the archive root, sha256 sidecar) — the required leg, the MT5 host runs
  Windows; `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` tar.gz legs (Wine hosts,
  best-effort, marked so in docs).
- `mise.mt5.toml` (host-only, `MISE_ENV=mt5`, separate file for the mise.host.toml reason):
  `[tools."http:mt5-bridge"]` with the windows-x64 URL, no rename_exe (exe name matches the tool key).
- `bridge/host-install.ps1` revised: the python tool step is dropped; `MISE_ENV=mt5 mise install`;
  the schtasks entry becomes a bare exe path (`/SC ONLOGON` initially, ONSTART-vs-ONLOGON and the
  System-account env path verified on the real host at phase 09).
- `docs/mt5-bridges.md` updated: the Rust binary is the bridge; the Python file is retired only in
  08-03, after the contract suite passes.

## Dependencies

Workspace-pinned only: axum 0.8, tokio, reqwest 0.12 (rustls), serde/serde_json (float_roundtrip
on), chrono, plus crate-scoped `subtle` (constant-time compare) and `dotenvy` (bridge.env). The
MCP client is hand-rolled on reqwest; no MCP SDK. `mcp-proxy` (joshrotenberg) was verified and
rejected 2026-11-26: a many-to-one gateway with a Tower middleware stack, no documented upstream
`Mcp-Session-Id` handling; it would add a middleware platform to a single-purpose binary whose
client is ~150 lines. MT5 data reaches the binary only through the terminal's MCP server; no
MetaTrader SDK, no third-party gateway.

## Tradeoffs accepted

- Five files instead of one; every trace stays three files deep. The mcp.rs quarantine exists so
  the riskiest code (session reopen, SSE scan, ms slicing) unit-tests without routes or pollers.
- Three MCP sessions kept (not one pooled client): the Python invariant survives verbatim.
- A Python fake in the contract suite: one interpreter on the dev box, in exchange for an
  independent framing implementation.
- arm64 ships untested against a real terminal (marked best-effort).

## Open questions (carried into plans)

1. Stale-session behavior of the real terminal (404 vs hang on a dropped session): the in-crate
   tests cover both; the contract suite picks the fake's behavior; the real host verifies at
   phase 09's checkpoint.
2. `time_ms` format assumption (chars 20..23 are the ms digits): documented in term.rs, covered by
   two slice tests; the real-host observation at phase 09 is the final oracle.
3. ONLOGON vs ONSTART for the schtasks entry (System account changes which bridge.env resolves):
   resolved on the real host in phase 09.
