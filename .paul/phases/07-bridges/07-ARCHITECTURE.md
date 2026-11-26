# Phase 07 architecture: per-user MT5 bridges

Contract for plans 07-01 to 07-04. The design source is the spec: the "MT5 bridges" section
(`.paul/design/specs/vtradingview.md:345-365`), CUJ 7 (`:171-179`), G6 (`:71-72`), the security plan
(`.paul/PLANNING.md:200-208`) and the phase gate R24 (readiness 14). This file turns those paragraphs
into types, signatures and module boundaries. Where the spec leaves a surface open, the decision is in
"Load-bearing decisions", recorded as **proposed D-032 / D-033** for the plan approval to settle; the
rejected alternative is named so the next reader does not re-litigate it.

## Problem

A user runs `bridge/mt5_bridge.py` on the machine that holds their MetaTrader 5 terminal, publishes it
over HTTPS with their own tunnel, and registers the URL plus a bearer token in Settings. The hub server
then calls that user-supplied URL: the classic SSRF position, made worse by multi-tenancy (G5: no data
crosses between users). The server must refuse every private target, pin the address it vetted, scope
every bridge to its owner, and keep the anonymous contract surfaces byte-identical (G1).

## Facts (measured 2026-11-26, branch arch/axum-tauri at 033f45a)

- `crates/hub-core/src/routes/ws.rs:42` `parse_subscribe(text)` reads `source`/`symbol`/`tf` (defaults
  binance/XAUUSD/M1); any unknown source becomes Binance (`crates/hub-types/src/market.rs:24`,
  `Source::from_wire(Some("mt5")) -> Source::Binance`). The test at `ws.rs:187` already pins the
  `{"source":"mt5","symbol":"BTCUSD","tf":"H1"}` shape, so the parity test will catch a broken default.
- `crates/hub-core/src/catalog.rs:65` `symbols()` builds Bybit + Deriv + Binance and the constants
  `mt5: Off, arm: Off` (D-003). `/api/symbols` is synchronous and anonymous today.
- `crates/hub-core/src/routes/market.rs:25` `bars()` is the chart-history handler;
  `crates/hub-core/src/routes/me.rs:16,38` show the `Viewer` extension pattern for session routes.
- `crates/hub-core/src/config.rs:47,97,215,271`: `VTV_SECRET_KEY` parses to `SecretKey` (at least 32
  random bytes base64url, `Debug` redacted). hub-core already depends on `sha2`, `hmac`, `base64`,
  `getrandom`; `reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }`
  is a workspace dep (`Cargo.toml:19`). `chacha20poly1305` is the only new dependency.
- Migrations live in `crates/hub-core/migrations/` (`0001_users`, `0002_user_state`); own runner, plain
  DDL, ids and times from Rust (D-011); single-table SQL (D-014).
- `bridge/mt5_bridge.py` (301 lines) serves `/health`, `/symbols`, `/bars`, `/wait` on `127.0.0.1:8797`
  with no auth; `TF_SEC = {M1, M5, M15, M30, H1, H4, D1}`; `/wait` long-polls up to 20 s; `/bars` rows
  are `{time (UTC s), open, high, low, close, volume}` with volume = tick volume float.
- The avatar menu is built by `hub/auth.js` (the `item(...)` helper, `hub/auth.js:281`);
  `hub/lab.js:131-141` fetches `/api/symbols` and already matches `s.source === "mt5"`.
- The three engine pages (`hub/vela.html`, `hub/lightweight.html`, `hub/klinechart.html`) hold the ws
  subscribe send sites.
- `.paul/runs/gate_all.py` takes `--pages` and diffs receipts against the manifest's page list (R-9);
  `ops/deploy.sh:8,52` writes `hub/SOURCE.txt` inside the working tree (DEBT-032's subject).
- Z5 ("start only after Phase 04 has live users"): the phase 07 transition ran 2026-11-26 and the live
  site stays v0.1.1 by owner decision. Surfaced for the plan approval to confirm; not silently assumed.

### Learnings inherited

Validated rows carried as plan Facts: L-003 (mutations at observable guards), L-007 (drive the real UI
served, read the live log), L-009 (grep a mutation landed before trusting a pass), L-013 (touch restored
files so cargo rebuilds), L-023 (quote exit status bare), L-031 (diff the tree against the plan before
executing), L-037 (in-page fetch carries `__Host-` cookies where `ctx.request` does not).

Candidate rows promoted into this phase's Verification, by plan:
07-01: L-001 (tokio Instant), L-008 (test state without file side effects), L-016 (in-memory source
under start_paused), L-039 (both engines: read the ignored count). 07-02: L-032 (utf-8 pinned in Python
drivers), L-040 (checks self-terminate). 07-03: L-022 (test surfaces behind debug_assertions and inside
gate::area()), L-026 (--list the filter, count it), L-029 (driver builds the binary, refuses a taken
port), L-030 (a check needing a server joins a harness that starts one), L-034 (Git-Bash launch
pattern), L-038 (unique subs against the shared dev db). 07-04: L-035 (static assets one path segment),
L-041 (addScriptTag for axe), L-043 (re-run surprising counts bare), L-036 (js-yaml for YAML).

## Shape

```
crates/hub-core/src/bridges/
  mod.rs      Bridges { client: BridgeClient } hung off AppState
  guard.rs    vet_url(raw: &str, auth_enabled: bool) -> Result<Vetted, Refusal>
              struct Vetted { url: Url (https), host: String, port: u16, addrs: Vec<IpAddr> }
              fn refused_reason(ip: IpAddr, auth_enabled: bool) -> Option<&'static str>
                  loopback | rfc1918 | cgnat | link_local_v4 | link_local_v6 | ula
                  | multicast | unspecified | v4_mapped  (table-driven, unit-tested)
                  loopback refused ONLY when auth_enabled (D-001/D-033)
              url rules: https only, no user info, port 443 or 1024..=65535
  crypto.rs   fn bridge_key(secret: &SecretKey) -> [u8; 32]   // HMAC-SHA256(secret, b"mt5-bridge-token")
              fn seal(key: &[u8; 32], token: &str) -> Vec<u8> // random 24-byte nonce || XChaCha20-Poly1305 ct
              fn open(key: &[u8; 32], blob: &[u8]) -> Result<String, CryptoError>
  client.rs   struct BridgeClient { http: reqwest::Client (redirects disabled),
                                     sems: Mutex<HashMap<i64, Arc<Semaphore>>> }
              trait Transport (test seam: in-memory transport, L-016)
              async fn health / symbols / bars / wait(...)
              enum BridgeError { Refused(&'static str), Dns, Cert, Auth, Timeout, TooLarge, Bridge(String) }
                  one variant per CUJ 7 cause; the variant's message reaches the UI verbatim
              per-bridge Semaphore(2) guards every call including /wait (spec: at most 2 concurrent)
              default call timeout 10 s; wait() overrides to 25 s (the bridge long-polls 20 s)
              response cap 5 MB (bounded body read); resolve -> vet -> connect to the pinned addrs only
  store.rs    Bridge::{insert, list_for(user_id), get_owned(user_id, id), delete_owned(user_id, id),
                       set_test_result(id, symbols_json, ok_at, error)}

crates/hub-core/migrations/0003_mt5_bridges.{up,down}.sql
  CREATE TABLE mt5_bridges (
    id INTEGER PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    url TEXT NOT NULL,
    token_enc BLOB NOT NULL,          -- 24-byte nonce || ciphertext
    symbols_json TEXT,                -- cached /symbols answer from the last successful test
    last_ok_at BIGINT,
    last_error TEXT,
    created_at BIGINT NOT NULL
  );
  CREATE INDEX idx_mt5_bridges_user ON mt5_bridges(user_id);

crates/hub-core/src/routes/mt5.rs     the /api/me/mt5* routes (below), registered inside the gate area
crates/hub-core/src/routes/mod.rs     mount
crates/hub-core/src/routes/me.rs      delete_me: verify the FK cascade covers bridges (G5/CUJ 8)
crates/hub-core/src/config.rs         VTV_LOCAL_BRIDGE_URL + VTV_LOCAL_BRIDGE_TOKEN, parsed only when
                                      AUTH_ENABLED=false (D-033); guard flag from auth_enabled
crates/hub-core/src/catalog.rs        symbols(state, viewer): anonymous shape unchanged; the session
                                      owner's bridge symbols appended (source "mt5", bridge id per entry)
crates/hub-core/src/routes/market.rs  bars(): source=mt5 + bridge=<id|local> -> bridge path, owner-checked
crates/hub-core/src/routes/ws.rs      parse_subscribe: optional "bridge" field -> Wanted { bridge: Option<i64> }
crates/hub-core/src/feeds/bridge.rs   BridgePoller per (bridge_id, symbol, tf): long-poll /wait, broadcast
                                      the changed candle to that want's subscribers, stop on last unsubscribe
bridge/mt5_bridge.py                  BRIDGE_TOKEN required (bearer, hmac.compare_digest), BRIDGE_BIND
hub/settings.html + hub/settings.js   the settings screen (mockup 03 is the normative layout)
hub/auth.js                           avatar menu entry "MT5 bridges" (signed-in only)
hub/lab.js + the three engine pages   bridge symbols flow into the picker; subscribe carries bridge
docs/mt5-bridges.md                   user setup: tunnel or Caddy, token, registration, error causes
```

### Route table

| Route | Auth | Behavior |
|---|---|---|
| `POST /api/me/mt5` | session | body `{label, url, token}`; url vetted (syntax + guard) then live-tested (`/health` then `/symbols`); 201 `{id, label, url, symbol_count, connected}` or 422 `{error: <exact cause>}` |
| `GET /api/me/mt5` | session | own bridges only: `{id, label, url, connected, last_ok_at, last_error, symbol_count}`; the token never leaves the server |
| `POST /api/me/mt5/:id/test` | session | re-test, refresh `symbols_json` + status; 200 with the result or 502 `{error}` |
| `DELETE /api/me/mt5/:id` | session | 204; another user's id answers 404 (G5; the admin-gate convention: outsiders get 404, never 403) |
| `GET /api/bars?source=mt5&bridge=<id>&symbol=&tf=&limit=&to=` | session (or local bridge when auth off) | owner-checked; maps the bridge's `/bars` rows onto the existing BarsResponse; errors carry the exact cause |
| ws subscribe `{"source":"mt5","bridge":"<id>","symbol":"X","tf":"H1"}` | session | want keyed `(mt5:<bridge_id>, symbol, tf)`; poller shared by that user's open charts |
| `GET /api/symbols` | session optional | anonymous response unchanged (G1); a signed-in owner's bridge symbols appended with `source: "mt5"` and `bridge: <id>` |

## Load-bearing decisions (proposed at plan approval)

1. **D-032 (proposed): the bridge surface contract.** (a) The ws subscribe JSON gains an optional
   `bridge` field; `source=mt5` without `bridge` keeps today's Binance fallback, so D-009 parity and the
   anonymous surfaces are preserved by construction. (b) `/api/symbols` stays synchronous: the bridge's
   `/symbols` answer is cached into `mt5_bridges.symbols_json` at every successful Test and merged for
   the session owner only; no live bridge call sits in the anonymous picker path. (c) Live updates poll
   `/wait` server-side, one poller per `(bridge_id, symbol, tf)` (the spec says "bridge and symbol"; the
   bridge's own `/wait` is keyed by symbol + tf, so tf joins the key). (d) The bridge's `/bars` row maps
   onto `Bar` field-for-field; tf values outside `TF_SEC` are rejected with the unknown-timeframe error.
   (e) Test and call failures surface the exact cause (refused / dns / cert / wrong token / timeout /
   too large / bridge error), verbatim into the UI (CUJ 7). (f) Cross-user access is 404. (g) No new
   rate bucket: the per-bridge 2-call semaphore plus the single shared poller is the "stricter per user
   on MT5" bound the spec names.
   *Rejected:* a separate `/api/me/mt5/symbols` endpoint feeding the picker client-side (two picker code
   paths for one list); ticker-encoding the bridge id (tickers reach the chart libraries and stay
   user-visible); a new per-user rate bucket (the semaphore is the bound the spec fixes).

2. **D-033 (proposed): the local-mode bridge and the loopback exception.** D-001 and the spec sentence
   ("loopback bridge URLs allowed only when auth is off") need a live mechanics: local mode
   (`AUTH_ENABLED=false`, no database, no users) gains `VTV_LOCAL_BRIDGE_URL` + `VTV_LOCAL_BRIDGE_TOKEN`
   env keys exposed as the implicit bridge id `local` on `/api/bars` and the ws subscribe. With auth on,
   loopback, private and link-local are always refused. This is also the integration-test seam: the
   served driver (07-03/07-04) runs an auth-off server against a loopback fake bridge.
   *Rejected:* dropping the exception (D-001 names it); allowing loopback for admins (no decision
   supports it).

3. **Token storage follows the spec exactly: XChaCha20-Poly1305 under `VTV_SECRET_KEY`.** The key is
   `HMAC-SHA256(VTV_SECRET_KEY, "mt5-bridge-token")` (the R21 key-separation pattern from phase 03; the
   cookie-MAC key is not reused). A random 24-byte nonce per record, stored prepended to the ciphertext.
   *Rejected:* reusing the session key directly; plaintext storage; a new config key (the SecretKey
   already exists and is Debug-redacted).

4. **The poller's timeout overrides the client default.** The bridge long-polls `/wait` for up to 20 s,
   which a blanket 10 s timeout would kill. `BridgeClient::wait` uses 25 s total; every other call keeps
   the spec's 10 s. The `/wait` call spends one of the bridge's two concurrency permits, so a `/bars`
   fetch may wait for a long-poll to finish; the spec fixes both numbers and they compose.

## Plan map

- **07-01 (wave 1, autonomous):** migration 0003, store, token crypto, SSRF guard, BridgeClient; unit
  tables and the mutation proof. Consumes: the SecretKey, reqwest, the migration runner.
- **07-02 (wave 1, autonomous):** `bridge/mt5_bridge.py` hardening (BRIDGE_TOKEN, BRIDGE_BIND),
  `docs/mt5-bridges.md`, and the phase's scheduled debt discharge (DEBT-032, DEBT-035). Independent of
  07-01: the bridge's route shapes are frozen by today's file.
- **07-03 (wave 2, depends on 07-01's Bridges/store/guard/client + migration):** the `/api/me/mt5*`
  routes, the symbols merge, the bars path, the ws bridge field + poller, the local-mode bridge, the
  SSRF and cross-user suites, logbus redaction of bridge tokens.
- **07-04 (wave 2, depends on 07-03's route contract):** the settings screen (mockup 03), the avatar
  menu entry, the picker wiring, the gate receipts, the served driver, and the owner checkpoint
  (DEBT-039, filed at plan time: a real bridge streams candles for its owner; no script here can hold an
  MT5 terminal).

## Security posture

The plan touches a trust surface (new endpoints, tokens, SSRF): `/paul:security` runs in guidance mode
on 07-01 and 07-03; the phase override `/security-review` loads at APPLY for every plan in this phase;
the phase exit runs it `--full` under the transition gate (`security_audit.enabled: true`,
profile standard). Redaction extends to bridge URL query strings and bridge tokens (spec `:371`).
