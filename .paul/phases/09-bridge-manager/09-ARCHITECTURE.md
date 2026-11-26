---
description: "vtradingview phase 09: the bridge manager app, the OTP pairing surface, and the permanent host install"
type: Architecture
about: "vtradingview"
---
# Phase 09 Architecture: the bridge manager app
**Status:** contract for plans 09-01..09-03 (written at plan time 2026-11-26, approved with the plans)
**Prior contract:** `.paul/phases/08-rust-bridge/08-ARCHITECTURE.md` (the binary this phase installs and controls)
**ROADMAP gate:** OTP pairing must not weaken D-032's auth (codes expiring, single-use, session-scoped;
URL + token over HTTPS only); the app's own capabilities stay as locked down as the v0.2 shell's.

## Problem
The bridge binary (phase 08) still needs hands: a quick tunnel that dies on reboot (DEBT-043), two
ONLOGON tasks, and credentials handed over a console file. Phase 09 replaces all of it with a second
Tauri app running on the MT5 VPS: install or repair the whole stack, pair itself to the owner's hub
account with a code shown in Settings, control and watch the services. The hub grows one pairing
surface; the v0.2 desktop shell and the bridge's frozen 4-route surface (D-032) do not change.

## Shape
```text
crates/mt5-manager/           SECOND tauri workspace (own Cargo.lock, like crates/desktop; D-030's
  Cargo.toml                    pattern: never a root member, own CI legs in desktop.yml)
  tauri.conf.json             local UI (frontendDist "ui"), identifier com.vtradingview.bridgemanager
  capabilities/manager.json   the LOCAL window: start/stop/restart commands only, no fs/shell/open
                              plugins to the webview; the Rust process owns every system call
  ui/index.html + ui/manager.js   the panel (hub theme variables, keyboard-complete)
  src/main.rs + lib.rs        tauri builder; commands module below
  src/commands/
    install.rs    install_stack(): mise install mt5-bridge + cloudflared (MISE_ENV=mt5), bridge.env
                  generation (random BRIDGE_TOKEN), named tunnel (login/create/route dns/config.yml),
                  the two schtasks (ONSTART + restart-on-failure), start + /health verify
    pair.rs       pair(url, token, code) -> POST {hub}/api/me/mt5/pair; keeps {id, revoke_code}
    control.rs    svc(start|stop|restart, bridge|tunnel) via schtasks /Change + /Run /End; taskkill
                  fallback (kill_fake_mcp.ps1's Get-CimInstance pattern, L-034)
    status.rs     local /health (bearer), schtasks query state, public-edge /health through the
                  tunnel URL (what vtradingview.com's side of the edge sees)
    revoke.rs     unregister(id, revoke_code) -> POST {hub}/api/me/mt5/paired/{id}/revoke
  src/runner.rs               the CommandRunner seam: every command runs through a trait so the
                              domain tests inject recorded outputs (L-016 analog: no real schtasks
                              or sockets under test)
hub (routes/pairing.rs)       the pairing surface, plan 09-01:
  POST /api/me/mt5/pairing-code     session-gated; mints a 10-digit code (rejection-sampled from
                                    getrandom, grouped "12345 67890"), TTL 10 min, bound to the
                                    minting session's user; multiple live codes allowed
  POST /api/me/mt5/pair             code-authenticated (no session): burns the code, resolves the
                                    user, then runs register's exact flow (vet_url auth-on ->
                                    /health -> /symbols -> seal -> insert); 201 {id, revoke_code};
                                    unknown/expired/reused code -> 401 with the desktop-exchange
                                    refusal texts; vet refusal -> 422 exact cause (CUJ 7 verbatim)
  POST /api/me/mt5/paired/{id}/revoke   revoke_code-authenticated: burns it, deletes that bridge
                                    row (owner-checked); wrong/reused code deletes nothing (G5)
migrations/0004_pairing.up    pairing_codes (code_hash PK, session_id_hash FK sessions ON DELETE
                              CASCADE, expires_at) + mt5_bridges.revoke_hash bytea NULL
```
The pairing codes reuse the desktop_auth_codes mechanics (S11/S12 burn-before-check, expiry sweep
cascade, CodeHash-style typed hashes in ids.rs); the wrong-guess hash never matches a row, so
guessing burns nothing — the protection is the per-IP bucket (D-007: 20/s) against the 10^10 space
over a 10-minute TTL. `revoke_hash` lives on the bridge row: one secret per bridge, shown once.

## Decisions
1. **The pairing endpoint is code-authenticated, not session-authenticated** (the VPS app has no
   browser session). It reuses register's vet/test/seal/insert unchanged, so the SSRF guard, the
   live test order and the sealed storage are identical on both paths (the gate's "not weaken
   D-032" holds by construction; a mutation proves the guard runs on the pair path).
2. **Unregister-from-app is a per-bridge revoke secret, not the pairing code** — the code is
   single-use and dies at registration; a second single-use secret (revocable, per-bridge) gives
   the app a standing unregister capability that cannot list, read or test any other bridge.
3. **The manager app's webview stays as locked down as the v0.2 shell's** (ROADMAP gate): the
   capabilities file grants the local window only the app's own commands; every schtasks, taskkill,
   mise and cloudflared call happens in Rust behind CommandRunner, never via a shell plugin.
4. **Install and repair are the same code path** (install_stack is idempotent: existing tasks are
   Set- not Register-, existing tunnel is reused, bridge.env is not overwritten) — the accept's
   "repair" is the same function run on a half-broken host.
5. **The named tunnel (mt5.vtradingview.com or the owner's subdomain) absorbs DEBT-043**: tunnel
   login/create/route dns need the owner's Cloudflare account, so those steps are owner-run with
   agent-written commands; the schtasks hardening to ONSTART + restart-on-failure is the app's job.
6. **Real-host facts stay checkpoints, not assumptions** (08-ARCHITECTURE's deferred list):
   stale-session behavior, `time_ms` ms digits, ONLOGON-vs-ONSTART, and the mise leg's real
   install are verified on 103.93.92.204 in 09-03; each has the command to run and the output to
   paste back.
7. **Distribution**: release.yml gains a windows-x64 installer leg for the manager (unsigned beta
   while Q4 stands, the DEBT-036 posture); 09-03's host work starts from a dev-built binary and
   the tag's artifacts take over after.

## Out of scope (this phase)
- Hub-side UI beyond the pairing-code button in Settings (09-01): no new page, no new gate page.
- Multi-host management, remote control of one VPS from another, any agent/daemon on the hub.
- Order placement or any trading action (PROJECT.md out of scope, unchanged).
- Code signing (DEBT-036), the v0.2 desktop shell's surface (untouched).
