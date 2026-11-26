---
phase: 09-bridge-manager
plan: 03
type: Summary
about: "vtradingview"
---

# 09-03 SUMMARY — distribution, the real host, and the four checkpoints

**Plan:** `.paul/phases/09-bridge-manager/09-03-PLAN.md`
**Status: PARTIAL — APPLYING, not applied.** The agent-side half is done and green. The owner-run
half (the install on the real host, the reboot, the four checkpoint observations) has not run:
it needs an RDP session on 103.93.92.204, the owner's Cloudflare account and the live Exness
terminal, and the agent has none of those. That half is `DEBT-047`, filed `blocking = "phase-09-exit"`.

## What shipped (agent-side)

- **`release.yml` gains a `manager` job.** `windows-latest`, `cargo tauri build --features gui` with
  `working-directory: crates/mt5-manager`, producing `mt5-manager-vX.Y.Z-windows-x64.zip` (portable
  exe at the archive root) plus `-setup.exe` (the NSIS installer) and a sha256, attached to the tag
  release. Signing is guarded exactly as desktop.yml's installers job guards it: an absent
  `WINDOWS_CERTIFICATE` is a clean skip, never a red run, and `certificateThumbprint` stays out of
  `tauri.conf.json`. The five v0.2 bridge/hub legs are untouched (`bin:` count still 5).
- **`bridge/host-install.ps1` rewritten for durability.** ONLOGON → **ONSTART** with a SYSTEM
  principal; `RestartCount 3`, `RestartInterval` 1 minute, `StartWhenAvailable`,
  `ExecutionTimeLimit 0`; the bridge task now carries **no argument** and the env file is mirrored
  beside the exe. It stays the repair path beside the app, and both write the same files and tasks.
- **`docs/manager.md`** (new): install, pair, control, status, unregister, the two scheduled tasks,
  and a troubleshooting table keyed on the real failure texts. **`docs/mt5-bridges.md`** now names
  the manager as the supported install path and the script as the manual/repair route.

## A fourth defect, found by the owner on the real host: missing `WebView2Loader.dll`

The owner ran the built app on the MT5 VPS and Windows refused to start it: **`WebView2Loader.dll`
missing**. Real, reproducible, and my earlier "the launcher starts a process" check was a **false
green** — this dev box happens to carry a copy of that DLL on `PATH` (found under
`C:\Program Files\Cloudflare\Cloudflare WARP\` and the Windows Performance Toolkit), so the app
started here and nowhere else.

**Root cause is the toolchain, not a packaging slip.** `webview2-com-sys-0.39.1/src/lib.rs:14-24`
links the loader **statically** only under `target_env = "msvc"`, and **dynamically**
(`WebView2Loader.dll`) on every other target. This box is pinned to
`x86_64-pc-windows-gnu` (rustc 1.95.0), so the exe imports the DLL. CI's `windows-latest` resolves
the MSVC host, so the release leg never sees this — only a GNU-built local install does, which is
exactly what the owner was handed.

**Evidence, both directions:**
- `objdump -p mt5-manager.exe` lists `DLL Name: WebView2Loader.dll` in the import table.
- The old zip held `mt5-manager.exe` alone. Extracted and run with the DLL removed from `PATH`, it
  exits **`-1073741515` = `0xC0000135` = `STATUS_DLL_NOT_FOUND`** — the owner's exact error.
- With the DLL copied in, the same extracted copy starts and shows the `BridgeManager` window.

**Fixed:** `release.yml`'s packaging step now copies `WebView2Loader.dll` into the portable zip when
the build produced it, prints the archive entries, and **throws** if the DLL was built but is absent
from the zip — a release can no longer publish an unstartable app. Re-run locally: the zip now lists
`mt5-manager.exe` + `WebView2Loader.dll`, and the extracted copy starts with a clean `PATH`.

**Not fixed, filed as DEBT-048:** the **NSIS installer** from a GNU build still omitted the DLL
(scanning the `setup.exe` bytes found no reference to it). `tauri.conf.json` deliberately got no
`resources` entry for it: a hardcoded `target/release/...` path fails the MSVC CI leg, where that
file does not exist.

**Resolved instead of patched, by the owner's direction: the repo now builds MSVC (D-038).**
`rust-toolchain.toml` pins `channel = "1.95.0-x86_64-pc-windows-msvc"` — the triple, not just the
version — because a channel-only pin resolves against rustup's `default_host_tuple`, which was
`x86_64-pc-windows-gnu` on this box. (A `target = "..."` key is silently ignored by rustup; the
triple has to be inside `channel`.) With msvc the loader links statically, so the DLL question
disappears for every local build and matches what CI always produced. Measured after the switch:
the exe drops 22.7 MB → 12.7 MB, `objdump -p` lists **no** `WebView2Loader` import, and the extracted
zip copy starts with a stripped `PATH` and no DLL beside it. Gates re-run under msvc via `justrust`:
root workspace 204 passed / 1 ignored, manager 78 passed. DEBT-048 now guards the pin itself —
it goes red if anyone reverts to a channel-only or gnu pin (verified both ways).

## Panel reordered to numbered steps + the MCP address made first-class (owner feedback)

The owner reported the five zones "confused". They were ordered Pairing → Status → Control → Install
→ Unregister: step 2 before step 1 (you cannot pair a bridge that is not installed), with the
terminal Unregister action sitting above Install. `<main>` is now `<ol class="steps">` in operating
order — **1 Install → 2 Pair → 3 Check → 4 Control → 5 Unregister** — with headings phrased as
actions and the lede saying "five steps, top to bottom".

Two defects surfaced while doing it:

1. **The step number was heading decoration as first written.** Inside each `<h2>`,
   `design_lint` called it "one coloured word in a heading" (dark theme only, which is how a
   colour-reading lint gives itself away). Moved into a `.step-head` flex row beside a clean `<h2>`;
   both themes now report `no findings`. The `<ol>` carries position for assistive tech, so the badge
   is `aria-hidden`.
2. **The badge's light-theme contrast was 3.72:1** — the same AA trap already recorded for
   `button.primary` (`--accent-ink` #ffffff on `--accent` #b8761a). Overridden to `--text` = 4.62:1;
   dark measures 8.34:1.

**And the MCP address, which the owner asked about while reviewing:** the MCP endpoint is
`http://127.0.0.1:22346/mcp` — confirmed live, not inferred: this box has the terminal's MCP
listening on `127.0.0.1:22346`, and an unauthenticated `POST /mcp` answers
`HTTP/1.1 401` with `Server: MetaTrader5-MCP`, `WWW-Authenticate: Bearer realm="MetaTrader5-MCP"`,
`MCP-Protocol-Version: 2025-06-18`.

What that check exposed: `bridge.env` carried **no `MT5_MCP_URL`** (neither
`install.rs~ensure_env` nor `host-install.ps1` wrote one), so the only address ever used was the
compiled default in `crates/mt5-bridge/src/main.rs:75`. A terminal on another port was unfixable
from the app. And the bridge answers `/health` even when the terminal's MCP is dead — the oracle's own
behaviour — so "installed" could mean "healthy bridge that can never fetch a bar".

Fixed, both parts:

- `InstallPaths`/`ensure_env` take an `mcp_url`, write `MT5_MCP_URL` explicitly on a fresh install
  (never relying on the compiled default), and **update** it on a repair when the operator names one
  — the address is configuration, while the two tokens stay byte-identical (`a_repair_corrects_a_moved_mcp_port_without_touching_the_tokens`).
  `normalize_mcp_url` accepts only what the terminal can serve — `http`, loopback host, `/mcp` path
  (a trailing slash normalizes), explicit port — and names the shape when it refuses.
- A sixth install step, **`MT5 MCP endpoint`**, probes the terminal directly through the `Http` seam
  and reports the measured status (`reachable: answered 401`) via a new `StepOutcome::Observed`, kept
  distinct from `Skipped` so the panel does not paint a working endpoint as a warning.
- The panel gained the **MT5 MCP address** input (`#mcp-url`, default `http://127.0.0.1:22346/mcp`);
  `host-install.ps1` prompts for it too and writes it into `bridge.env`.

Live negative control: the same probe against `127.0.0.1:22399` returns no connection
(`curl` code `000`), which is the `did not answer` branch — so the check can fail.

Gates: manager 84 tests (7 new), clippy `ok: lint succeeded` (plain and `--features gui`), fmt 0,
UI 10/10 both themes with receipts in `.paul/ds-receipts/09-03/`, msbuild rebuilt exe 12.7 MB, zip
repackaged and the extracted copy starts with a stripped `PATH` and no DLL.

## Auto-check of the MCP address and token + install progress (owner request)

Two owner requests, both about step 1 being blind: (a) nothing verified that the MT5 MCP **token**
was right — `probe_mcp` sent no credential, so a stale token passed install and failed at the first
chart; (b) clicking Install gave no progress and named no file, because the whole pass is one
blocking call.

**(a) `check_mcp`** — an authenticated MCP `initialize` (`Http::post_json_auth`, a new seam method so
pair/revoke cannot silently start carrying a bearer). The verdicts separate the two failures the
operator must fix differently: **401 with the token sent = the address is right, the token is not**
(copy it again); 2xx (or a 400/406/422 carrying a JSON-RPC error, which some builds answer to a
`protocolVersion` they dislike after authenticating) = the pair works; transport error/404 = the
address. Exposed as the `check_mcp` command behind a **Check address and token** button, and run by
install itself as its last step ("MT5 MCP address and token"). `McpCheck::Ok(200)` measured live
against the real terminal (serverInfo `MetaTrader 5 MCP`), wrong token measured 401.

**(b) progress without an event bus.** `core:event:*` is forbidden by AC-2, so push is out; the
design is pull: `install_stack_reporting` reports a `Progress {index,total,label,detail}` snapshot
through a `ProgressSink` into `AppState`, and the panel polls the new `install_progress` command every
400 ms — a determinate bar (`aria-valuenow`), the step label, and the file detail. For the two mise
steps the detail is **mise's own last output line** (`download_detail`), which names the actual asset
(`mt5-bridge-v0.3.0-windows-x64.zip`) — the honest answer to "which file", never guessed. A failed
run writes `Progress::with_label("Stopped: …")` so the bar never hangs mid-flight.

**ACL:** the capability grows from five to seven `allow-*` commands (`check-mcp`, `install-progress`),
both read-only probes; the pin test asserts seven with the justification in a comment, and the
capability description now says seven.

**A clean-checkout trap the rebuild exposed:** `tauri-build` validates the capability against
`permissions/autogenerated/*.toml` **before** `generate_handler!` compiles, and those committed files
listed only the five original commands — so a clean clone failed `build.rs` with
`Permission allow-check-mcp not found`. Fixed the Tauri-documented way: the two generated tomls
(`check_mcp.toml`, `install_progress.toml`) are committed; the macro owns them from then on. The
release workflow (fresh runner) would have hit exactly this.

## Three defects found while wiring Task 1
Each was real on the host path, not stylistic. All three are pinned by tests that fail without the
fix.

**1. The bridge task pointed at a file the binary never reads (would have broken AC-2 and AC-3).**
`install.rs` registered `VTVBridge` with `--config "<deploy root>\bridge\bridge.env"`, but
`crates/mt5-bridge/src/main.rs:141` parses **no** command-line arguments at all — it loads
`bridge.env` from the directory of its own exe. `mise which mt5-bridge` resolves into mise's store,
so the task would have started, found no `BRIDGE_TOKEN`, logged `BRIDGE_TOKEN is required` and
exited 1: a bridge the panel reports as installed and that never answers `/health`.
Fix: `InstallPaths::env_beside_exe` mirrors the deploy-root env beside the resolved exe on every
install (which also restores it after `mise install` replaces that directory), and the task is
registered bare.
Pinned by `the_task_gets_an_env_file_the_binary_can_actually_read`. Mutation: disabling the mirror
turns exactly that one test red (`env mirrored beside the exe`), restoring it goes green — 1 red, no
collateral.

**2. "Restart-on-failure" was a comment, not a setting.** The registration ended with `/F`, described
as "keep running on failure". `/F` means force-overwrite; `schtasks /Create` has no restart flag, and
`/Change` cannot set it either. AC-2 asks for it explicitly.
Fix: a second, argv-only call —
`Set-ScheduledTask -TaskName $env:VTV_TASK -Settings (New-ScheduledTaskSettingsSet -RestartCount 3
-RestartInterval (New-TimeSpan -Minutes 1) -StartWhenAvailable -ExecutionTimeLimit ([TimeSpan]::Zero))`
— with the task name travelling in `VTV_TASK`, never concatenated into the script text (the S1
lesson from 09-02's security review).
Pinned by `every_registered_task_gets_restart_on_failure_settings`: two tasks ⇒ exactly two settings
calls, names carried in the environment, and the script text containing neither task name.

**3. `host-install.ps1` carried defects 1 and 2, plus ONLOGON.** Rewritten to ONSTART/SYSTEM with
the settings object and the env mirror, so the script and the app cannot drift into installing
different hosts.

## Debt discharged (the plan's own requirement)

The plan requires at least one discharge (pile-up was 16 against a budget of 12); it got two.

| Row | What discharged it |
|-----|--------------------|
| **DEBT-046** | D-035 (`py_str` reproduces Python `str(float)`) and D-036 (`term.rs` takes `&M: Terminal`) transcribed into `.paul/DECISIONS.md` in their phase-08 wording. The id list is now contiguous `D-001..D-037` (37 entries, no gaps). |
| **DEBT-045** | `diff_bridge_contract.py` gained a `--normalize` mode (reads one JSON body on stdin, pops `pollers` and `measured_at` — the same two keys its own `normalize` already defines); `check-mt5c.sh`'s inline logged-out `/health` comparison now delegates to it instead of re-implementing the canonicalization. The suite exits **0** for the first time: `DIFFER: 33/33 cases identical, 0 differences` twice, then `MT5C-OK both passes identical, 0 differences`. |

**R-10 honoured:** editing the checker invalidates the phase-08 mutation results, so
`mutate-mt5c.sh` was re-run after the edit — `MUTATIONS-OK 6 red / 6 green, final suite green`.

Negative control on the new helper: two bodies differing only in `measured_at`/`pollers` normalize
identically, and two differing in `server` still differ — so the fix cannot pass vacuously.

## Gates (all run bare, after the last edit)

| Gate | Result |
|------|--------|
| `cargo test --manifest-path crates/mt5-manager/Cargo.toml --all-targets` | 78 passed, 3 suites (was 74) |
| `cargo clippy … --all-targets -- -D warnings` | no issues |
| `cargo check … --features gui` | resolves and compiles |
| `cargo fmt … --check` | exit 0 |
| `cargo test --workspace` | 204 passed, 1 ignored, tauri-free |
| `npx -y js-yaml .github/workflows/release.yml` | exit 0 |
| PowerShell parser on `bridge/host-install.ps1` | 0 errors |
| `bash .paul/runs/check-mt5c.sh` | exit 0, 33/33 both passes |
| `bash .paul/runs/mutate-mt5c.sh` | `MUTATIONS-OK 6 red / 6 green` |
| `debt.py --run-checks` | 0 overdue; 15 waiting (DEBT-047 added, 045/046 done) |

## Acceptance criteria — where each stands

| AC | State |
|----|-------|
| **AC-1** the manager distributes | **satisfied on the dev box, artfact-at-tag still to observe.** The leg exists, names the manager workspace, requires `--features gui`, parses under js-yaml (0). Beyond parsing, the workflow's own commands were **run locally**: `cargo tauri build --features gui` in `crates/mt5-manager` produced `mt5-manager.exe` (22.7 MB) and `bundle/nsis/BridgeManager_0.3.0_x64-setup.exe` (5.41 MiB), the launcher was observed to start a process and shut down cleanly, and the packaging step verbatim produced `mt5-manager-v0.3.0-windows-x64.zip` (8.5 MB, exe under the single folder entry, matching the bridge legs' archive shape), `-setup.exe` and `.zip.sha256`. The no-`gui` binary refuses with `mt5-manager: built without the 'gui' feature` and **exit 2**, as AC-1 claims. All five pwsh blocks parse (0 errors). Only artifact-attachment on a real tag is unobserved (the DEBT-038 first-push pattern). |
| **AC-2** the real host runs the permanent stack | **not satisfied — blocked on DEBT-047.** The code now sets ONSTART/SYSTEM/restart-on-failure and mirrors the env the binary reads, but nothing has run on 103.93.92.204. |
| **AC-3** pairing reaches production end to end | **not satisfied — blocked on DEBT-047.** |
| **AC-4** the four phase-08 checkpoints | **not satisfied — blocked on DEBT-047.** See the section below; it is deliberately empty. |
| **AC-5** docs and debt close | **docs half done** (`manager.md` written, `mt5-bridges.md` updated; DEBT-046 + DEBT-045 discharged). The DEBT-043 half is void: the owner dropped that row on 2026-11-26 (a quick tunnel's temporary domain is acceptable), and the AT-STARTUP hardening it also asked for now lives in `install_stack`, which is what this plan wired. |

## Real-host checkpoints

**Not yet recorded — this section is the open half of the plan and the discharge target of
DEBT-047.** Each line below is filled from an observed output plus the command that produced it
(R-7), never from an assumption.

- [ ] **(a) Stale-session behavior** — drop the MCP session on the real terminal; record whether the
      bridge answers 404 or hangs, and how it reopens. Command: force a session drop, then
      `curl -s -i "http://127.0.0.1:8797/bars?symbol=XAUUSDm&tf=M1" -H "Authorization: Bearer $TOKEN" | head -1`
- [ ] **(b) `time_ms` ms digits from the real terminal** — whether chars 20..23 carry milliseconds or
      the documented correction applies. Command: a `/bars` sample against the real terminal.
- [ ] **(c) ONSTART vs ONLOGON under the SYSTEM/service context** — which trigger survives a reboot
      with no interactive logon. Command: `schtasks /query /tn VTVBridge /v` before and after a reboot.
- [ ] **(d) the mise leg's real install** — `MISE_ENV=mt5 mise install mt5-bridge` fetching a
      published artifact on the host (the 08-03 deferred half).

Public probes that must answer from the dev box once the host is up: `/health`, `/bars XAUUSDm M1`,
`/wait` through the tunnel URL.

## What the owner runs (DEBT-047)

**Ordering caveat, found while writing the docs:** there is no `v0.3.0` tag yet (tags stop at
`v0.2.0`), so the manager is not downloadable from a release today. Until the tag is pushed, build
it on a Windows box with a Rust toolchain and WebView2 — `cargo tauri build --features gui` in
`crates/mt5-manager` — and copy `target/release/mt5-manager.exe` (portable) or
`target/release/bundle/nsis/BridgeManager_0.3.0_x64-setup.exe` to the host. Both were built and
verified on the dev box during this plan.

1. On 103.93.92.204 (RDP): run the manager installer or the portable exe from the release.
2. `cloudflared tunnel login` once in a browser on that host (authorises the domain).
3. In the app: **Install or repair**, pasting the MT5 MCP token (*Tools > Options > MCP*).
4. Paste back `schtasks /query /tn VTVBridge /v` and the tunnel config path.
5. One reboot; confirm both tasks ran with no logon and the public `/health` still answers.
6. Mint a pairing code in Settings, pair in the app, watch XAUUSDm M1 stream, then unregister from
   both sides.

## Deviations recorded

1. **The plan's Task 1 text says only "switch both tasks to ONSTART"**; making AC-2's
   restart-on-failure actually true needed the second `Set-ScheduledTask` call, because no
   `schtasks` flag provides it. Recorded rather than silently accepted as "done".
2. **The plan passes `--config` to the bridge task implicitly** (it inherited `install.rs`'s existing
   registration). That argument is inert on the real binary, so the plan's intent — a task that
   finds its env — is met by the file mirror instead.
3. **DEBT-043's discharge was dropped**, so the plan's Task 3 "flip DEBT-043 to done" has no target.
   The hardening it also requested is delivered by `install_stack` (ONSTART, no time limit,
   restart-on-failure), which is what AC-2 tests.

## Next

The owner's host run (DEBT-047), then Tasks 2 and 3 close, then UNIFY. The tree stays uncommitted
until the phase transition (the session-reset rule), as with 09-02.
