# Milestones

Completed milestone log for this project.

| Milestone | Completed | Duration | Stats |
|-----------|-----------|----------|-------|
| v0.1 Hosted launch | 2026-11-25 | 3 days (11-23 → 11-25) | 4 phases, 13 plans |
| v0.2 Accounts everywhere | 2026-11-26 | 2 days (11-25 → 11-26) | 3 phases, 12 plans |

---

## ✅ v0.1 Hosted launch

**Completed:** 2026-11-25
**Duration:** 2026-11-23 → 2026-11-25 (3 days)

### Stats

| Metric | Value |
|--------|-------|
| Phases | 4 (01 Rust parity, 02 order flow, 03 auth, 04 deploy) |
| Plans | 13 |
| Public site | https://vtradingview.com (live 2026-11-25, v0.1.1) |

### Key Accomplishments

- Rust (Axum) hub replaces the Python server with wire parity: contract suite diffs raw JSON
  key sets and ws shapes; order flow (delta, footprint, session profile) from fixtures within
  tolerance, no numpy/pandas/pyarrow.
- Google sign-in: first sign-in creates one user, the second reuses it, sessions revocable,
  admin gate 404s outsiders; per-page hashed CSP, four rate buckets, TTFC beacon.
- Public deploy through a Cloudflare Tunnel (no inbound ports), backups + restore drill
  rehearsed, AGPL source offer complete (notices.html + /SOURCE.txt name the exact commit).
- First green CI (seven jobs) and the browser-gates job running served-page checks on runners.

### Key Decisions

D-001 local mode for charts only · D-002 websocket protocol kept (one subscription per socket)
· D-005 axum-0.8-compatible pins, Cargo.lock is the pin · D-007 in-crate token bucket (20/s,
burst 40) · D-010 CSRF by Origin + SameSite=Lax, no token · D-011 own migration runner over
pgsqlite.

---

## ✅ v0.2 Accounts everywhere

**Completed:** 2026-11-26
**Duration:** 2026-11-25 → 2026-11-26 (2 days)

### Stats

| Metric | Value |
|--------|-------|
| Phases | 3 (05 synced state, 06 Tauri desktop, 07 MT5 bridges) |
| Plans | 12 |
| Files changed | 227 across the three phase commits (48 + 74 + 105) |
| Live proof | sync across browsers; desktop window checkpoint-approved; real Exness bridge streaming XAUUSDm |

### Key Accomplishments

- Synced state per user (G4): four /api/me/state routes, hub/sync.js, a signed-out layout
  appears in a second browser after sign-in, concurrent edits 409 with both values recoverable
  (harness 19/19, check-0503 22/22).
- Desktop shell (Tauri 2): the installed app signs in through the system browser
  (`vtradingview://` hand-off), the remote origin holds ONE permission, nav guard refuses
  off-origin, tray Show/Quit; desktop.yml CI green on its first run.
- Per-user MT5 bridges (G6): sealed token storage (XChaCha20-Poly1305), SSRF guard vetting
  resolved addresses (DNS rebinding dead via pinned connect), owner-scoped routes with cross-user
  404, owner-only symbols merge, per-(bridge,symbol,tf) refcounted poller, settings screen + picker
  wiring; DEBT-039 discharged 2026-11-26 — the owner's real Exness bridge streamed live XAUUSDm
  candles through the public tunnel.
- Phase reflects ran at every phase close (pstack panels); the v0.2 arcs produced R-12
  (automated falsifiers before human gates), D-034 (parallel_agents false here), and the
  L-024 discharge-record anchor rule after four self-matching-check false greens.

### Key Decisions

D-026 no IndexedDB mirror · D-029 desktop deep link by Rust-side eval (remote origin gets ONE
permission) · D-030 crates/desktop is its own Cargo workspace · D-032 the bridge surface contract
(optional ws bridge field, owner-only symbols merge, per-want poller, no new rate bucket) · D-033
local mode carries one env-configured bridge (loopback only when auth is off) · D-034 plans emit
parallel_agents false in this repo.

---
