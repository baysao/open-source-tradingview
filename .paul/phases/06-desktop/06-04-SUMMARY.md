# 06-04 SUMMARY — CI, unsigned installers, the missing docs, and the phase-06 debt rows

**Applied:** 2026-11-25. **Plan:** `.paul/phases/06-desktop/06-04-PLAN.md` (wave 4, the last plan of
phase 06). All five tasks executed; the closing human-verify checkpoint was answered **approved** by
the owner — owner looked at the window: done (2026-11-25).

## What shipped

- **`.github/workflows/desktop.yml`** (new): `desktop` workflow, `on` push (main, arch/axum-tauri) +
  tags `v*` + pull_request, **no `paths:` filter** (AC-1). `check-windows` (windows-latest) runs
  `cargo test --manifest-path crates/desktop/Cargo.toml --all-targets` + clippy `-D warnings`;
  `check-linux` (ubuntu-latest) installs the four Tauri v2 packages (`libwebkit2gtk-4.1-dev
  libappindicator3-dev librsvg2-dev patchelf`) and runs the same test. `installers` (tag-only,
  matrix ubuntu/windows): tauri-cli via `taiki-e/install-action`, `cargo tauri build`, artefact
  upload, and a `gh release view || gh release create` step whose notes carry "unsigned beta" so the
  words cannot drift out. Signing posture while Q4 is open: `WINDOWS_CERTIFICATE` /
  `WINDOWS_CERTIFICATE_PASSWORD` wired at job env, import + signtool steps guarded by
  `if: env.WINDOWS_CERTIFICATE != ''` (an absent secret is a clean skip); no
  `certificateThumbprint`/`signCommand` in `tauri.conf.json` (untouched).
- **`.github/workflows/ci.yml`**: one line added to the `fmt` job
  (`cargo fmt --manifest-path crates/desktop/Cargo.toml --check`). Every other job byte-identical;
  `cargo test --workspace` stays tauri-free (D-030).
- **`docs/api.md`** (new): the `/api/me/state` contract transcribed from
  `crates/hub-core/src/routes/state.rs`'s module doc (the code is the source): the four calls with
  success bodies and literal error strings, the three un-inferable rules (`If-Match: 0` = insert;
  key charset 1..=128 bytes of `A-Za-z0-9_-.:`; the list omits values on purpose), 401-before-key-check
  on all four routes, and the measured caps (256 keys / 256 KiB, D-024; the key cap corrective, D-031).
- **`docs/desktop.md`** (new): the sign-in flow (system browser because Google refuses OAuth in
  embedded webviews; deep link carries the code; exchange; single-use with 60 s of life, D-015), the
  exhaustive capability table (one permission, `allow-start-sign-in`; D-029's eval rationale), and
  building it (MSVC with the measured gnu-host reason, `cargo +stable-x86_64-pc-windows-msvc tauri
  build`, the four Linux packages, D-030's root-workspace exclusion, the unsigned-beta installer posture).
- **`.paul/design/specs/vtradingview.md`**: the two stale claims amended, each marked "(amended in
  Phase 06, D-029/D-030)": the Desktop shell paragraph now says one permission + Rust-side eval, and
  item 46 now says `window.vtvDesktop` (was `window.__TAURI__`).
- **`.paul/DECISIONS.md`**: D-029 (one permission, not the spec's two), D-030 (crate outside the
  root workspace, own CI job), D-031 (the state cap stays corrective) — appended in the D-028 house
  shape. `.paul/STATE.md` decisions table updated to match.

## AC results

| AC | Result | Evidence |
|---|---|---|
| AC-1 desktop crate built/tested on every push | **files proven, CI run unproven** | YAML valid (js-yaml exit 0); no push was possible this session — the tree is uncommitted by project convention (`auto_commit: false`; the phase commit happens at transition). Reported unproven per the plan's own escape hatch, not asserted. |
| AC-2 root gates unchanged | **PASS (by construction)** | The only ci.yml change is the fmt extension; the `test` job and every other job are byte-identical in the diff. The log check (no `tauri`/`webkit` in the root test job) rides on the same unproven push and is reported with AC-1. |
| AC-3 tag builds unsigned installers | **files proven, tag build unproven** | Workflow shape + the "unsigned beta" release-note step; no tag push this session. |
| AC-4 both docs pages exist and are true | **PASS** | `grep -c "/api/me/state" docs/api.md` → 5 (≥4); all seven error strings grepped out of `state.rs` (`not found` 4, `version conflict` 2, `value too large` 2, `too many keys` 2, `bad key` 4, `if-match required` 2, `bad if-match` 3); docs/desktop.md greps 3 (unsigned beta / MSVC command / permission id); spec amendments count 2. |
| AC-5 debt rows speak the truth | **PASS** | `debt.py --run-checks`: DEBT-033 and DEBT-034 `OK done`; DEBT-036 `waiting` (owner), DEBT-037 discharged at checkpoint approval; no phase-06 row left `open`. `debt.py --gate phase-06-exit` → clear, exit 0. |

## The DEBT-033 repair, proven

The old check (`grep -c 'D-026' .paul/DECISIONS.md`) was vacuous: D-026 already existed as the
unrelated phase-05 "no IndexedDB mirror" decision, so the row passed on day one. Negative control
run as the plan requires: with the check temporarily flipped back to `'D-026'`, `debt.py
--run-checks` reported DEBT-033 `OK done` — the false pass, observed. Restored to
`grep -c 'D-031' .paul/DECISIONS.md` (expect 1): `OK done`, now only because the D-031 entry exists.
The finding is recorded verbatim in the row's note.

## Deviations

1. **Permission literal:** the plan's Task 3 verify pattern said `allow-start_sign_in` (underscore);
   the real permission id is `allow-start-sign-in` (hyphen) —
   `crates/desktop/permissions/autogenerated/start_sign_in.toml:4`. The docs state the true string
   and the verify grepped the hyphen form. Writing the plan's literal would have put a false
   identifier in an auditor-facing page.
2. **CI-run evidence unproven (AC-1/AC-3 remote halves):** no push in this session (uncommitted
   tree, `auto_commit: false`). The plan explicitly sanctions reporting this unproven rather than
   asserting it. First push of the branch will prove both.
3. **Execution note (not a plan deviation):** three write-time defects in `desktop.yml` were
   self-caught and fixed before verification: a wrong env var in the certificate-import step,
   `$OS` instead of `$RUNNER_OS` in the Linux artefact copy, and a placeholder literal where
   `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` belongs.

## Deferred rows (a view of the store, not the store)

- **DEBT-036** `waiting` (owner): Q4 code signing — a Windows EV/OV certificate and an Apple
  Developer ID are missing; installers stay unsigned beta until wired (secrets-only change; the
  guarded env names are already in `desktop.yml`). Check greps `docs/desktop.md` for the signed
  posture, which the unsigned doc must not contain yet (the DEBT-033 trap, avoided).
- **DEBT-037** `done` (2026-11-25): the native-window UAT discharged by the approved checkpoint —
  owner looked at the window: done.
- **DEBT-032** left untouched on purpose: its check is a path on the production host
  (`/app/vtradingview/repo/hub/SOURCE.txt`) that does not exist here, so it reads `waiting` and can
  only be verified on the host. Its fix already landed in `ops/write_source.sh` plus
  `ops/deploy.sh:56` and the manual sequence at `ops/README.md:140`; the open question is
  verification, not code.
- **DEBT-035** (auth-off gate base note in `gate_all.py`) and **DEBT-016/017** remain the agent
  rows they were; DEBT-033/034 are this plan's discharges.

## Skill audit

| Skill | Invoked | Notes |
|---|---|---|
| /signing-tauri-apps | yes | apply-start load (plan-blocking); the env-var names and the no-`certificateThumbprint` rule shaped the installer job |
| /configuring-tauri-capabilities | yes | apply-start load; the capability posture section of docs/desktop.md |
| /technical-writing | n/a (optional) | house style taken from `docs/vela-storage-keys.md` |
| /paul-security | n/a | optional per-task; a phase-exit report per the Phase-03 precedent, not per plan |
| /design-gate | receipts checked here | `gate_all.py --pages hub.html,index.html --check-receipts` → 8/8; the full gate set runs at UNIFY |
| /reflect (pstack) + /rsi | pending | run at the phase UNIFY |

## Issues encountered

- rtk's grep rewrite returned a false `0` for `grep -c "/api/me/state" docs/api.md`; bare grep
  (RTK_OFF=1) returned 5. Command-output claims above were re-run bare where a count looked wrong.
- The plan's Task 3 verify pattern literal (underscore form) did not match the shipped permission
  id — see Deviation 1.

## Next-phase readiness

Phase 06 is APPLY-complete (4 of 4 plans: 06-01..06-04). Next: the phase UNIFY — reflect (pstack) +
rsi, the design-gate set, and the phase commit (the tree holds 06-01..06-04's work, uncommitted).
The owner decision stands: **do not deploy**; the live site stays v0.1.1 and any deploy is a
separate, explicit step.

## Reflect and RSI

- reflect: reflect (pstack panel, phase scope over the 06-01..06-04 SUMMARYs) — 6 accepted findings:
  L-024 bumped (the vacuous-check class gains the pre-existing-subject mechanism, DEBT-033's day-one
  pass), L-007 hits 3 and flips **validated** (the served/live pass caught 06-01's three live-only
  defects and 06-03's two render lies while unit checks stayed green), L-036 bumped (js-yaml again),
  new L-043 (rtk can falsify a count — re-run bare), new L-044 (derive Tauri permission ids from the
  generated ACL, never the plan's literal), and DEBT-038 filed (the desktop CI run is unobserved;
  first-push proof). Rejected: a ZCode worktree-isolation rule (already covered by the apply
  workflow's sequential fallback), merging L-043 into L-023 (different mechanism). No reusable-skill
  edits proposed; everything routed to the .paul/ stores. Note: the three-lens panel ran inline
  (the harness's async agents were cancelled by a session break and its TaskOutput retrieval is
  defective), per the reflect skill's no-subagent fallback; the digest carried the four SUMMARYs and
  fresh parent-independent artifacts. rules: +0, decisions: +0 (D-029/030/031 were logged at APPLY),
  learnings: +2 added, 3 bumped, 1 promoted, debt: +1 added, 3 discharged (033/034/037).
- instruction edits proposed: none
- rsi: no candidate needing a before-and-after run — L-043/L-044 were verified by direct check (the
  bare re-run; the ACL grep), and the store's only experiment-bearing row (L-037) was validated with
  its experiment in phase 05 (`.paul/experiments/L-037.md`).
- ledger: added hub/hub.html (appearance ledger, huashu row; the `hub.html`-at-root spelling fails —
  the delivered page is `hub/hub.html`).
