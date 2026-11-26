# Plan 08-03 Summary: parity proven, distribution live, the Python oracle retired

**Plan:** `.paul/phases/08-rust-bridge/08-03-PLAN.md` (approved 2026-11-26, option 1)
**Applied:** 2026-11-26T18:50+07:00
**Status:** APPLIED — 4 of 4 tasks PASS; the phase's accept is met: 33/33 contract cases
identical across both framing passes, six mutations red-then-green, the Python bridge deleted.

## What shipped

| File | Contents |
|------|----------|
| `.paul/runs/fake_mcp_terminal.py` | the stdlib fake MCP terminal: POST /mcp only, session map with `Mcp-Session-Id` responses, the five tools with deterministic grid data (6000-row cap, above the 5000 clamp), ERRX isError symbol, control file flipping `logged_in` / `sse` / `drop_session` (440 = stale session → the client must reopen), token-env refusal at start, self-terminating (L-040). |
| `.paul/runs/diff_bridge_contract.py` | the differ: 33 cases (/health, /symbols, /bars × 7 TFs × 4 limit/to variants, /wait after="" and after=<current key>, the ERRX 502); normalizes only `pollers`/`measured_at`; canonical JSON on everything (array order preserved); exits 1 on any difference. |
| `.paul/runs/check-mt5c.sh` | the suite driver: ephemeral ports (L-029), PID files + taskkill fallback (L-034), trap cleanup, SKIP_BUILD=1, three passes (plain JSON, SSE, logged-out errors), R-8 bare exits via `set -uo pipefail`. The Python oracle's copy lives at `.paul/runs/0803/mt5_bridge_oracle.py` (git history holds the original) so the parity diff stays runnable after the retirement. |
| `.paul/runs/mutate-mt5c.sh` | the six-mutation driver (L-003/L-009/L-013/R-10): exact-index replace, landing grep, red run with the exit quoted bare, restore grep, green re-run, final full suite. |
| `.github/workflows/release.yml` | five-leg matrix: vtv linux x64+arm64 (unchanged), `mt5-bridge` windows-x64 (required; zip + sha256, exe at the archive root, pwsh packaging), `mt5-bridge` linux x64/arm64 (best-effort tarballs). js-yaml parses. |
| `mise.mt5.toml` | `[tools."http:mt5-bridge"]` pinned 0.3.0, windows-x64 asset URL, no rename_exe (the exe name matches the tool key). |
| `bridge/host-install.ps1` | rewritten: the python step is gone; `MISE_ENV=mt5 mise install mt5-bridge` from the deployed `mise.mt5.toml`; ONLOGON scheduled tasks as bare exe paths (VTVBridge, VTVTunnel; ONSTART-vs-ONLOGON stays open for phase 09's host checkpoint); bridge.env generation and the tunnel flow kept. `install.cmd` comment updated. |
| `docs/mt5-bridges.md` | rewritten for the binary: mise install, env contract unchanged, linux legs marked best-effort, failure-mode table kept. |
| `bridge/mt5_bridge.py` | **deleted** (AC-5); `__pycache__` cleaned; no repo surface names the file (grep over docs/ bridge/ scripts/ ops/ = 0 hits). |

## AC results

| AC | Verdict | Evidence |
|----|---------|----------|
| AC-1 fake terminal | PASS | live probes: initialize returns `Mcp-Session-Id: fake-session-N`; tools/call answers grid bars; SSE pass via the control file; ERRX `isError` with the fixed text; `drop_session` → 440; logged_out → balance-only account; no `MT5_MCP_TOKEN` refuses with exit 1. |
| AC-2 contract parity | PASS | `SKIP_BUILD=1 bash .paul/runs/check-mt5c.sh` → `DIFFER: 33/33 cases identical, 0 differences` on the plain pass and the SSE pass, `MT5C-OK`, exit 0; the logged-out `/health` canonical compare identical. |
| AC-3 mutations | PASS | `bash .paul/runs/mutate-mt5c.sh` → `MUTATIONS-OK 6 red / 6 green, final suite green`; L-026 `--list` pin 40 asserted before the runs; every landing/restore grepped; exits quoted bare. |
| AC-4 distribution | PASS (to the dev box's limit) | js-yaml parses release.yml (exit 0); the release-profile binary builds and refuses without a token (exit 1); `MISE_ENV=mt5 mise install` resolves the config and requests exactly the workflow's asset URL (404 until the next tag publishes it — the same first-push pattern DEBT-038 recorded; the attach step is the standing check). The same http backend installs the proven vtv asset on this box (mechanism verified). |
| AC-5 retirement | PASS | `git rm bridge/mt5_bridge.py`; grep finds no `mt5_bridge` reference outside `.paul/` history; docs describe only the binary. |

## Deviations (each with its reason)

1. **The fake's grid cap is 6000 rows, not 4000.** At 4000 the Python oracle's resync loop kept
   widening its span past 30 y on the `limit=99999` cases and died on Windows'
   `datetime.fromtimestamp` (`OSError: [Errno 22] Invalid argument`) for pre-1970 windows — a
   real oracle behavior, but one the real terminal never reaches (its history is far denser).
   6000 rows keeps both bridges inside one widening step, so the diff tests the contract, not a
   Windows platform artifact. The Rust `iso()` clamps negatives to epoch and would have masked it.
2. **`/bars` diffs canonically, not as raw bytes.** The architecture's "raw bytes for /bars"
   assumed one serializer; Python's `json.dumps` inserts spaces Rust never would, and the hub
   (D-032's consumer) parses JSON and never compares strings. Canonical JSON (array order
   preserved, `sort_keys` on objects only) diffs the values; the float round-trip formatting is
   pinned by 08-01's `py_str` tests.
3. **The differ's `normalize` canonicalizes lists too** (the first run's `/symbols` diff was a
   list falling through to raw text).
4. **The suite launches the retired oracle from `.paul/runs/0803/mt5_bridge_oracle.py`** (a
   copy of the deleted file), so parity remains checkable after AC-5; the header comment says so.
5. **m6's mutation marker is `// MUT-WD`** — the first restore check used `return Ok(snap);`,
   which also exists in the original code, so the restore looked unfixed. A unique marker fixed
   the check, not the code (the code was correctly restored; verified by the green re-run).

## Verification (bare exit statuses, R-8)

- `SKIP_BUILD=1 bash .paul/runs/check-mt5c.sh` → exit 0, `DIFFER: 33/33 ... 0 differences` × 2 passes, `MT5C-OK`
- `bash .paul/runs/mutate-mt5c.sh` → exit 0, `MUTATIONS-OK 6 red / 6 green, final suite green`
- `cargo fmt --all --check` → exit 0; `cargo clippy --workspace --all-targets -- -D warnings` → exit 0
- `cargo test --workspace` → 200 passed, 1 ignored, exit 0; 0 tauri/webkit/wry lines
- `cargo build --release -p mt5-bridge` → exit 0; the release binary prints the Python refusal line and exits 1 without a token
- `npx -y js-yaml .github/workflows/release.yml` → exit 0
- `MISE_ENV=mt5 mise install` → resolves the config, requests the workflow's exact asset URL (404 pending the next tag)
- `grep -rn mt5_bridge docs/ bridge/ scripts/ ops/` → 0 hits

## Deferred

- The mise leg's end-to-end proof (a real install of a published `mt5-bridge-vX.Y.Z-windows-x64.zip`
  on a Windows host) lands with the next tag's release run; release.yml's attach step and phase
  09's host checkpoint observe it. Not a debt row: the check is the workflow run itself.

## Next

Phase 08's plans are all applied and unified; the transition (phase commit, ROADMAP complete,
next-phase routing) is the phase close's work. Phase 09: the Tauri manager app (install, OTP
pairing, control, status) on the MT5 VPS, and the real-host checkpoints this phase deferred
(stale-session behavior, `time_ms` ms digits, ONLOGON-vs-ONSTART, the mise leg).

## Reflect and RSI

- reflect: deferred to phase close (reflect: pstack) — pending, runs in the fresh session (session_reset: plan)
- instruction edits proposed: none
- rsi: deferred to phase close (reflect: pstack)
