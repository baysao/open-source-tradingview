---
description: "vtradingview: milestone and phase structure"
type: Roadmap
about: "vtradingview"
---

# Roadmap: vtradingview

## Overview

Move TradeX from a local Python hub to a hosted, multi-user service at `https://vtradingview.com`, with a
Rust (Axum) server, Google accounts that sync each user's work, and a Tauri 2 desktop shell. Brief:
`.paul/design/specs/vtradingview.md`.

## Current Milestone

Run /paul:discuss-milestone or /paul:milestone to define the next one.

## Completed Milestones

<details>
<summary>v0.1 Hosted launch - 2026-11-25 (4 phases, 13 plans)</summary>

| Phase | Name | Plans | Completed |
|-------|------|-------|-----------|
| 01 | Rust workspace and hub-server parity | 3/3 | 2026-11-23 |
| 02 | Order flow in Rust | 3/3 | 2026-11-23 |
| 03 | Database and Google sign-in | 3/3 | 2026-11-24 |
| 04 | Public deploy | 3/3 | 2026-11-25 |

</details>

<details>
<summary>v0.2 Accounts everywhere - 2026-11-26 (3 phases, 12 plans)</summary>

| Phase | Name | Plans | Completed |
|-------|------|-------|-----------|
| 05 | Synced state | 4/4 | 2026-11-25 |
| 06 | Tauri desktop | 4/4 | 2026-11-26 |
| 07 | Per-user MT5 bridges | 4/4 | 2026-11-26 |

Full details: `.paul/MILESTONES.md`, archives: `.paul/milestones/`.

</details>

## Milestones

| Milestone | Phases | Goal |
|-----------|--------|------|
| v0.1 Hosted launch | 01 to 04 | the public site with live charts and Google sign-in |
| v0.2 Accounts everywhere | 05 to 07 | synced state, desktop app, per-user MT5 |

**Launch note:** the hosted site stays at v0.1.1; promoting it to the v0.2 build waits on the owner
rows (data terms Q2, trademark Q3, privacy text Q9 — DEBT-020/026; DEBT-013 and DEBT-018 are
discharged).

## Phases

| Phase | Name | Milestone | Plans | Status | Completed |
|-------|------|-----------|-------|--------|-----------|
| 01 | Rust workspace and hub-server parity | v0.1 | 3 (01-01, 01-02, 01-03) | Complete | 2026-11-23 |
| 02 | Order flow in Rust | v0.1 | 3 (02-01, 02-02, 02-03) | Complete (reflect deferred) | 2026-11-23 |
| 03 | Database and Google sign-in | v0.1 | 3 (03-01, 03-02, 03-03) | Complete | 2026-11-24 |
| 04 | Public deploy | v0.1 | 3 (04-01, 04-02, 04-03) | Complete | 2026-11-25 |
| 05 | Synced state | v0.2 | 4 (05-01..05-04) | Complete | 2026-11-25 |
| 06 | Tauri desktop | v0.2 | 4 (06-01..06-04) | Complete | 2026-11-26 |
| 07 | Per-user MT5 bridges | v0.2 | 4 (07-01..07-04) | Complete | 2026-11-26 |

## Phase Details

Every phase plan first applies the design-doc fixes in its **Gate** row. The findings come from the EGM
review in `.paul/design/reviews/vtradingview/` (owner concluded each stage with them open, 2026-11-23);
the proposed but unapplied fixes are in `goldfish_review_report.md`. A gate is cleared when the spec says
the right thing, not when the code does.

### Phase 01: Rust workspace and hub-server parity
- **Build:** Cargo workspace (`hub-types`, `hub-core`, `hub-server`), static allow-list, existing `/api/*`
  routes except order flow, `/ws` on port 8770, Deriv, Bybit and Binance feeds through FeedHub, log bus,
  `Procfile` + `scripts/dev.sh` (`hub` line), CI.
- **Accept:** a contract suite that diffs raw JSON (key sets and types, websocket `type` values and field
  sets) passes against `python hub/server.py` and `hub-server`; `/.env` is 404; one port only.
- **Gate: cleared 2026-11-23** (D-001, D-002, spec Phase 1 facts rewritten against the code). Was: E1 self-hosting decision, and with it the `mt5`, `arm`, `ws_port` fields
  (R14); websocket protocol kept as today's one subscription per socket or redesigned (R1, N1); per-IP
  socket limit that fits an 8-chart layout (R2); facts about today's server corrected: log ring 12,000
  (R7), logbus and feedlog parity (R8), `/api/health` clock offset and lag (R9), `/` serves
  `hub/index.html` (R11), restart and journal routes (R12); contract oracle is raw JSON, not
  deserialisation (R31, N4); crate list (R17); CI live-upstream rule (R30); clippy command (R29).
- **Risks:** upstream bans of CI and data-centre IPs; protocol drift breaking the frontend.

### Phase 02: Order flow in Rust
- **Build:** delta, footprint and session volume profile ports, Binance USD-M futures trade archives and
  REST trades, the candle-estimate fallback, golden fixtures.
- **Accept:** every fixture matches within the stated relative tolerance; numpy, pandas and pyarrow are no
  longer needed by the hub.
- **Gate: cleared 2026-11-23** (spec Phase 2 facts and items 16 to 18 rewritten against the code). Was: cache
  is parquet under `hub/.cache/binance/` and `hub/.cache/rest_trades/` (R5): new format and path or real
  parquet; `tpo.rs` removed or specified, TPO is browser-side (R6); weight budget and constants (R10);
  deterministic fixture capture, `XAUUSDT` mapping (R32).

### Phase 03: Database and Google sign-in
- **Build:** migrations (users, identities, sessions, desktop codes), pgsqlite `db` line, One Tap and
  redirect sign-in, sessions, admin gate (`/admin/*`), account deletion.
- **Accept:** first Google sign-in creates exactly one user, the second reuses it; CI green on pgsqlite
  and Postgres 17; non-admins get 404 on every `/admin/*`.
- **Gate: cleared 2026-11-23** (spec "Phase 3 facts and decisions"; D-010 to D-013). Was: CSRF approach (C1: derived token, or drop the token per N5 with `Origin` required); full
  column tables with types, indexes and cascades (R3); request and response shapes for every new route
  (R4); session slide, cookie MAC, key separation, issuer rule (R21); `AUTH_ENABLED` behaviour (R18);
  log panel polling gated on admin status (R13); `sync.js` not loaded before Phase 05 (R27).
- **Plans (2026-11-24):** 03-01 config, db, migrations; 03-02 auth and routes; 03-03 frontend sign-in,
  log panel gating, browser check. Contract: `phases/03-auth/03-ARCHITECTURE.md` (D-014 to D-017).

**Phase 03 exit (2026-11-24):** all three plans unified. 03-01 config, schema and the migration runner;
03-02 the auth module, gate and route tables; 03-03 the header control, avatar menu, delete dialog and the
gated log panel. Gates: `cargo test --workspace` 118 passed, fmt and clippy clean in both feature ways,
the browser check 66/66 with four mutations each red then restored, axe 0 violations both themes. The
dev db now binds loopback (`pgsqlite --host`, DEBT-011 discharged). Still open: the first green CI run
(DEBT-005), the real-Google round trip (DEBT-010, blocked on DEBT-013), and the owner's UI look.

### Phase 04: Public deploy
- **Build:** production host, Cloudflare Tunnel `tunnel` line, rate limits, CSP and security headers,
  backups and restore drill, licence and AGPL source notices, privacy page, TTFC beacon.
- **Accept:** live charts through the tunnel; the host has no inbound ports; restore drill done; AGPL
  source link live; TTFC visible on the admin panel.
- **Gate:** Risks & Mitigations section in the spec (C2); CSP string that keeps the inline scripts,
  Google Fonts, Vela Workers and GIS working, and rate-limit values (R19); feed cap counts symbols and
  reserves the default chart (C14, N2); TTFC defined once and persisted (C16); backup destination (R26).
  Launch blocked on Q2, Q3, Q9.
- **Plans (2026-11-24):** 04-01 deploy surface (ops/, tunnel line, load test, .txt ext) UNIFIED 2026-11-24;
  04-02 security + metrics (hashed per-page CSP, four rate buckets, TTFC beacon and admin read,
  reserved default-chart key, DEBT-003 reconnect) UNIFIED 2026-11-25; 04-03 pages, docs, spec C2,
  Python retirement UNIFIED 2026-11-25.

**Phase 04 exit (2026-11-25):** all three plans unified. 04-01 the deploy surface; 04-02 the
hashed per-page CSP, the four rate buckets, the TTFC beacon and the reserved default chart;
04-03 the public pages, the served gate runner, the docs and spec C2, and the Python retirement
(CI proves the Rust-only tree). Gates this phase: check-0402 60/60 (reconnect included),
check-0303 66/66 under the shipped CSP, gate_all 8/8 receipts, m1-m8 and m1-m4 mutations
red-then-green, fmt/clippy/tests green throughout. Still open for the launch: the owner commit
(DEBT-024), the host and the real drill (018/019), Q2/Q3/Q9 (plus 020/021/026), the first green
CI run (005) and the 1,000-socket load target (022).
  Contract: `.paul/runs/architect-0400/SYNTHESIS.md` (arena of 2, corrections measured in code).

### Phase 05: Synced state
- **Build:** migration 0002 `user_state`; `/api/me/state` reads and a versioned `PUT` with `If-Match`
  (409 carries the current value); a daily job expiring 30-day-old backups; `hub/sync.js` with the storage
  wrappers, the iframe channel, per-key debounce and conflict states; the sign-in merge (server wins, the
  local value kept as a restorable backup); the Q5 key inventory.
- **Accept:** a layout drawn signed-out appears after sign-in in a second browser; a concurrent edit is
  refused with 409 and nothing is overwritten silently; clashing keys produce a backup and Restore works;
  cross-user reads and writes are impossible.
- **Gate:** Vela key inventory first (Q5); merge finishes before the iframe loads, merge writes excluded
  from upload (N3); per-tab versions (N7); `tradexDrawFavs` and `tradexFavBar` decision (R16); delete,
  key-limit, backup-collision and offline cases (R23); copy deck and states (C7, C13).
- **Gate status (planning, 2026-11-25):** Q5 **cleared by measurement** — `hub/vela.html:148` passes
  `persist: false`, so no Vela workspace key is written; `vela:` stays as a placeholder prefix and
  `docs/vela-storage-keys.md` records the absence. N3/N7 decided in `05-ARCHITECTURE.md` (one writer per
  tab; versions cached at `vtv:ver:<key>`, never uploaded). R16 cleared: both `tradex*` keys are
  allow-listed exactly, no wildcard. R23's edge cases are 05-01 AC-7 (key limit), 05-03 AC-8 and AC-10
  (backup collision, offline) and 05-02 AC-7 (delete). C7/C13 states are 05-03 AC-4..AC-11. One spec fact
  was found wrong and is carried as **D-024**: the 1 MB value is unreachable behind the gate's
  300,000-byte body cap, so the limit is 256 KiB.
- **Plans (2026-11-25):** 05-01 storage (migration, S15..S21, `Auth::state_*`); 05-02 HTTP
  (`/api/me/state`, the contract table, the prune job, the gate-test fix for templated paths); 05-03 client
  (`sync.js`, the channel, the merge, the notices, the harness); 05-04 the phase's debt (hub.html under the
  gate set, the dev db, SOURCE.txt, the store writeback). 05-04 is wave 1 because 05-03's gate evidence over
  `hub.html` is impossible until that page passes.
- **Status: ✅ Complete 2026-11-25** — 4/4 plans applied and unified. Accept evidence: a signed-out
  layout appears in a second browser after sign-in (harness CUJ3/CUJ4); a concurrent edit 409s with
  both values recoverable (CUJ5, A17/A18 cases); clashing keys back up and Restore works (AC-8);
  cross-user reads impossible (05-02's 401-before-key-check ordering). Verification at close:
  check-0503 22/22, harness 19/19, gate_all 8/8 + receipts, check-0402 56/56, CHECK_0303=1 → 66/66,
  workspace 139 passed, fmt+clippy clean, phase-05 debt gate clear. Reflect panel: 20 accepted
  (3 code fixes A15-A17, D-026/D-027/D-028, L-037 validated, 5 new candidate rows); rsi: 1 experiment
  run and passed (L-037).

### Phase 06: Tauri desktop
- **Build:** `crates/desktop`, deep-link sign-in handoff, tray, capability lock-down, installers.
- **Accept:** the installed app signs in through the system browser and shows the same synced layout.
- **Gate:** how the page gets or avoids `desktop_key` (K4, N6, R25); webkit2gtk in CI (R27); signing (Q4).
- **Planned 2026-11-25** (4 plans, `06-ARCHITECTURE.md` is the contract). The gate is answered:
  **N6 chosen by the owner** — `start_sign_in` returns the in-memory `desktop_key` to the page and the page
  POSTs the existing `/auth/desktop/exchange`, so the server surface is unchanged; K4 (Rust navigating the
  webview to a new GET route) was rejected because a navigation reloads the page and destroys the key the
  one-time code is bound to. R25 (capabilities) resolved to **one** permission for the remote origin
  (D-029): the deep link is delivered by a Rust-side `eval`, which needs no ACL entry, so the spec's "two
  permissions" shrinks by one. R27 (webkit2gtk) is confined to a dedicated Linux CI job because the crate
  is its own workspace (D-030). Q4 (signing) stays open by design: the installer job ships unsigned betas
  and the new debt row is its trigger.
  Plans: 06-01 shell + capability + guard + tray · 06-02 deep-link handoff (warm, queued, cold start) ·
  06-03 `hub/auth.js` desktop branch + the four-state overlay + driver · 06-04 CI, installers, `docs/api.md`
  and `docs/desktop.md`, and the phase-06 debt writeback (including repairing DEBT-033's vacuous check).

**Phase 06 exit (2026-11-26):** all four plans unified. 06-01 the shell + capability + guard + tray
(16 tests, 2 mutations, live CDP pass — three live-only defects found and fixed); 06-02 the
`vtradingview://` hand-off (31 tests; m4 exposed a vacuous test then bit; warm, queued and cold
paths proven over the real OS protocol); 06-03 `hub/auth.js` desktop branch + the four-state
overlay (33 driver assertions over 12 served cases, 3 mutations red-then-green, gate receipts 8/8);
06-04 CI (`desktop.yml`, `ci.yml` fmt covers the crate) + unsigned tag installers + `docs/api.md` +
`docs/desktop.md` + the debt writeback (DEBT-033's vacuous check repaired with a negative control;
034/037 discharged; 036 Q4 and 038 first-push proof open). Checkpoint approved 2026-11-25: owner
looked at the window. Still open after the phase: DEBT-036 (Q4 signing, owner), DEBT-038 (first CI
push, agent), and the launch-waiting owner rows.

### Phase 07: Per-user MT5 bridges
- **Build:** bridge token and HTTPS mode, `mt5_bridges`, SSRF guard, bridge routes, settings screen.
- **Accept:** a real bridge streams candles for its owner; every SSRF case is refused; no other user can
  see or use the bridge.
- **Gate:** how bridges reach the symbol picker and the subscribe format (R24); start only after Phase 04
  has live users (Z5).
- **Planned 2026-11-26** (4 plans, `07-ARCHITECTURE.md` is the contract). R24 answered by proposed
  D-032: the ws subscribe JSON gains an optional `bridge` field (`source=mt5` without it keeps the
  Binance fallback, so D-009 parity and the anonymous surfaces hold); the bridge's `/symbols` answer is
  cached at every successful Test and merged into `/api/symbols` for the session owner only; live
  updates come from one server-side poller per (bridge, symbol, tf) long-polling `/wait`; failures
  surface the exact cause verbatim (CUJ 7); cross-user is 404; no new rate bucket. Proposed D-033 gives
  D-001's loopback-when-auth-off exception its mechanics: local mode reads `VTV_LOCAL_BRIDGE_URL` /
  `VTV_LOCAL_BRIDGE_TOKEN` as the implicit bridge `local`, which is also the served-test seam. 07-02
  schedules the phase's debt discharge (DEBT-032, DEBT-035); DEBT-039 (owner real-bridge uat) was filed
  at plan time and blocks phase-07-exit. Z5 confirmed satisfied at the plan approval 2026-11-26
  (the live site serves v0.1.1; owner approval).
  Plans: 07-01 bridge core (migration, crypto, guard, client) · 07-02 Python bridge hardening + user
  docs + debt discharge · 07-03 routes, symbols merge, bars + ws poller, suites · 07-04 settings
  screen, picker wiring, gates, served driver, owner checkpoint.

**Phase 07 exit (2026-11-26):** all four plans unified; the loop closed at 07-04's UNIFY with the
phase reflect (pstack panel: 8 accepted, 1 rejected, 2 backlog — L-024/L-029 validated at hits 3,
L-027 generalized, L-048..050 added, D-034 parallel_agents-false, R-12 automated-falsifiers-first,
DEBT-040/041 filed). The build is proven: 313 workspace tests exit 0, m1-m3 red-then-green (07-03),
a11y mutation red-then-green (07-04), gate receipts 8/8 x 5 pages including settings.html,
check-0704 14/14 served, the SSRF 17-case table green auth-on and auth-off, cross-user 404s,
logbus redaction proven by the ECHO fake. **The phase's accept is discharged: DEBT-039 done
2026-11-26** — the owner ran the real bridge on the MT5 host (Exness-MT5Real28), registered it in
Settings, and watched live XAUUSDm candles stream; the agent verified the public half through the
tunnel (health/bars/wait/symbols all 200, 358 symbols). /security-review --full ran at the
transition gate: no Critical/High, 2 Medium + 1 Low filed as DEBT-042. Also open after the phase:
DEBT-038 (first-push CI proof, agent), DEBT-043 (permanent named tunnel + ONSTART tasks + the
registration-file credential handoff), DEBT-040/041 (agent), and the launch-waiting owner rows.

---
*Roadmap created: 2026-11-23*
*Last updated: 2026-11-26 after milestone v0.2*
