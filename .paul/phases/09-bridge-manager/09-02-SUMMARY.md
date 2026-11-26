---
phase: 09-bridge-manager
plan: 02
type: Summary
about: "vtradingview"
---

# 09-02 SUMMARY — the manager app

**Plan:** `.paul/phases/09-bridge-manager/09-02-PLAN.md` (applied 2026-11-26, one session,
sequential per D-034)
**Shipped:** `crates/mt5-manager` — the second Tauri workspace (D-030): pair / install / control /
status / revoke over 09-01's surface, every system call behind a seam, the panel in `ui/`, CI legs
in desktop.yml.

## What shipped

- **The workspace** (`crates/mt5-manager/`): own Cargo.lock, `[workspace]` table, never a root
  member. `tauri` + `tauri-build` are OPTIONAL behind a non-default `gui` feature, so the domain
  suite runs with no webkit/WebView2 installed (AC-1's "only the tauri binary needs webkit").
- **The seams** (`src/runner.rs`, `src/http.rs`, `src/secret.rs`): `CommandRunner` (argv vectors,
  never a shell string), `Http` (rustls, caller-supplied timeouts), `Fs` (bridge.env /
  config.yml in tests), `Delay` (the bounded health poll). All have `Fake` recorders; no test
  spawns a process or opens a socket (L-016, D-036).
- **The domain** (`src/commands/`): `pair` (HTTPS-only pre-check before any request, exactly the
  four contract keys, verbatim 401/422/502 causes + a next-action line), `revoke`, `control`
  (schtasks /Query-/Run-/End + the L-034 Get-CimInstance kill as a *fallback*), `status` (local
  /health with bearer + bounded deadline, task states, edge /health at the tunnel URL), `install`
  (mise → bridge.env kept-if-present → tunnel reuse-or-create → schtasks tasks → bounded /health
  poll).
- **The wiring** (`src/app.rs`, `gui`-gated): five commands, all `async` + `spawn_blocking` (a sync
  command runs on Tauri's MAIN thread — a 30 s pair would have frozen the window), in-memory
  secrets (`Secret`'s `Debug` prints a length, so no log line can leak a value).
- **The panel** (`ui/index.html` + `ui/manager.js`): one screen, hub theme values copied by value,
  native `<dialog>` for pair and revoke-confirm, revoke code shown once with a copy-on-gesture,
  no localStorage secrets, refresh gated at 15 s.
- **CI** (`desktop.yml`): both legs gain the manager's `cargo test --all-targets` + clippy
  (tauri-free by default); Linux additionally `cargo check --features gui` (no link, no webkit
  needed for that). js-yaml exit 0.

## AC results

| AC | Result | Evidence |
|----|--------|----------|
| AC-1 root tauri-free, manager tests webkit-free | pass | root `cargo test --workspace` exit 0, **204 passed**, `grep -ci "tauri\|webkit\|wry"` = 0; manager `cargo test` **74 passed** with `gui` off; manager crate has its own `[workspace]` |
| AC-2 capability lockdown | pass | `tests/capability.rs` (7 tests): exactly `allow-pair/install/control/status/revoke`, no `plugin:`/`core:`, `local: true`, NO remote block (contrast with hub.json), config's app/build/bundle name no remote URL, `withGlobalTauri: false`, identifier `com.vtradingview.bridgemanager` |
| AC-3 pair contract | pass | `pair.rs` tests: `http://` refused with 0 requests sent; body = exactly `{code,label,url,token}` (4 keys asserted); 401 "code unknown or expired" verbatim + next action; 422 guard text verbatim; grouped code normalised; revoke_code lives in `Secret` and a test proves `{:?}` prints no value |
| AC-4 runner seam + recorded tests | pass | start-on-running = no-op (1 call); restart = `/End` then `/Run` (order pinned, 4 calls); missing task surfaces the exact schtasks error and names install; `/health` asserts bearer header + 15×1s bound; edge check asserts the tunnel URL, refuses loopback and `http://` |
| AC-5 install idempotent | pass | second run: `bridge.env` NOT rewritten (written-paths asserted), token kept, tasks `Updated` (query → `/Create /F`), tunnel `Kept` from cloudflared's JSON, bounded poll 15×1s, no cert → skip naming `cloudflared tunnel login` |
| AC-6 panel renders clean | pass | 10/10 gate receipts (5 gates × 2 themes) in `.paul/ds-receipts/09-02/`, verify_responsive both themes (0 overflow, 0 covered, 320/768/1280), dialogs measured OPEN (focus lands inside, Esc closes), 0 console errors, look pass on both theme screenshots |

## Verification (the plan's list, run)

- `cargo fmt --all --check` root exit 0 + manager exit 0 (R-8 bare)
- `cargo clippy --workspace --all-targets -- -D warnings` root exit 0; manager own clippy exit 0
  plain AND `--features gui` exit 0 (the tauri wiring compiles; 213 crates)
- `cargo test --workspace` root exit 0 (204, tauri-free grep 0) + manager 74 (`--list` = 74)
- ui/ gates: receipts in `.paul/ds-receipts/09-02/` (10 JSON + shots), runner
  `.paul/runs/09-02-gates.py` (imports gate_all's harnesses; gate_all itself cannot serve a page
  outside hub/)
- `npx -y js-yaml .github/workflows/desktop.yml` exit 0

## Decisions made during apply

1. **`gui` is a non-default feature** (the mechanism AC-1 implies but Task 1 did not name): `tauri`
   and `tauri-build` are optional deps; `src/app.rs` + `main.rs`'s real body are `#[cfg]`-gated.
   `cargo test` compiles zero tauri; `cargo check --features gui` is the compile proof. Recorded
   here rather than as a deviation because AC-1 demands the property.
2. **schtasks `/Create /F` instead of PowerShell `Set-ScheduledTask`** for the install tasks: the
   PowerShell cmdlet needs a shell string, which Task 2 forbids. `/F` overwrites in place, so a
   second registration cannot happen; the query decides the *outcome* name (`Installed` vs
   `Updated`).
3. **The kill fallback is PowerShell with argv-passed image + needle** (L-034's Get-CimInstance
   pattern): the script text is a constant; inputs travel as `$args` elements, never spliced into
   the script (tested).
4. **Two extra seams beyond the plan's file list**: `Http` and `Fs` (the plan named only
   CommandRunner) — pair/revoke/status need HTTP and install needs the env file, and L-016 forbids
   sockets in tests; temp dirs would test real I/O but tie the suite to a filesystem. `Secret` is
   the third addition, making "never in a log line" a type property instead of a review rule.
5. **Light-theme primary-button ink deviates from hub/theme.js** (documented in the CSS): the
   preset's `accentInk #ffffff` on `accent #b8761a` measures 3.72:1 < AA at 14px. The graphite
   preset's dark-ink-on-amber pattern (--text) measures 4.62:1. This is the one place the panel is
   not a verbatim copy of the hub tokens.

## Deviations / additions to record

- Files beyond `files_modified`: `src/http.rs`, `src/secret.rs`, `src/view.rs` (IPC shapes, tested
  without gui), `tests/capability.rs` (mirrors desktop's), `icons/*` (copied verbatim from
  crates/desktop — bundle needs real files), `.paul/runs/09-02-gates.py`, `.gitignore` gained
  `crates/mt5-manager/gen/` (tauri-build's output, same as desktop's).
- `Cargo.lock` is generated and committed (own workspace, desktop's precedent).
- The window shows ~120 ms after build (visible(false) + delayed show). The native-window look —
  title bar, no tray — is not judgeable from a served page and joins 09-03's owner checkpoint.
- DEBT-046 was NOT folded in (owner chose plain apply): the pile-up stands at 16 vs budget 12, and
  the UNIFY rule's "next PLAN schedules a discharge" lands on the next plan written.

## Security review (phase-09 override, whole plan diff at UNIFY)

Focus per SPECIAL-FLOWS phase 09: body-secret authentication, long-lived secrets, and — new in
this plan — process spawning from a Tauri command.

| # | Sev | Finding | Disposition |
|---|-----|---------|-------------|
| S1 | High | `powershell -Command "<script>" <image> <needle>`: PowerShell concatenates every token after `-Command` INTO the script text, so `$args[0]` is never bound — the image name would have executed as its own statement and the kill fallback would never have matched. Fixed: params travel in the process environment (`VTV_IMAGE`/`VTV_NEEDLE`), the script reads `$env:`; test asserts no input value appears in argv and the argv shape is fixed at 4 | fixed, tests green |
| S2 | Medium | reqwest's default redirect policy follows up to 10 hops — a 302 on `/health` would re-send `Authorization: Bearer` to the redirect target. Fixed: `Policy::none()` on the client (these four endpoints never legitimately redirect) | fixed |
| S3 | Medium | `install(deploy_root)` from the webview reached the `/TR` quoted string of both scheduled tasks and every file write unvalidated — a quote or `|&<>` in the root breaks out of the quoting. Fixed: `validate_deploy_root` refuses quotes, shell metacharacters and control chars before any use; test enumerates the class | fixed |
| S4 | Low | `edge_health` accepted a userinfo URL (`https://user:pw@host`), sending the bearer there. Fixed: refused, parity with pair's `hub_origin` | fixed |
| S5 | Medium, accepted | The five IPC commands remain powerful by design (install writes files, control runs schtasks): the boundary is the capability file plus the page being app-shipped. A compromised manager webview equals a compromised operator session — the same trust model as 09-01's two-secret surface (B9). Matches D-030's threat model; no action | recorded |
| S6 | Low, accepted | Tasks registered `/RU SYSTEM /RL HIGHEST` — the host-installer hardening DEBT-043 asked for; the bridge must run headless at boot | recorded |

Re-verified after the fixes: 76 tests (69 lib + 7 capability), clippy both feature sets exit 0,
fmt exit 0. The UI gates are unaffected (no ui/ change).

## Deferred rows

None filed. The plan's scope held: no real-host execution (09-03), no release.yml change, no
tunnel credential handling (absent cert = an exact skip message naming the owner's command).

## Issues encountered

- Tauri sync commands run on the main thread (would have frozen the UI during install/pair) —
  found at wiring review, fixed with async + `spawn_blocking` before any gate ran.
- `tauri`'s IPC requires `Serialize` on every command return type — surfaced as an `IpcResponse`
  trait-bound error from `generate_handler`, fixed with derives.
- gate_all.py's static gates resolve pages under `hub/`, so the manager page needed the importing
  runner (`.paul/runs/09-02-gates.py`) rather than a gate_all invocation.

## Reflect and RSI

- reflect: deferred to phase close (reflect: pstack) — 09-02 is not the phase's last plan; the panel runs at 09-03's UNIFY over every SUMMARY in the phase
- instruction edits proposed: none
- rsi: deferred to phase close (reflect: pstack)
- ledger: skipped (not the phase's last plan; the rendered-surface ledger add happens at 09-03's UNIFY)

## Next-phase readiness

09-03 (install_stack on the real host 103.93.92.204) can start: the 09-01 pairing surface is
committed (`97d57c3`), 09-02's app builds and gates green. The owner will need: the MT5 MCP token
(Tools > Options > MCP), one `cloudflared tunnel login` in a browser on the host, and a hub
pairing code from Settings. The real-host facts ARCHITECTURE decision 6 defers (ONSTART + System
account behavior, mise's real install) are exactly what 09-03 verifies.
