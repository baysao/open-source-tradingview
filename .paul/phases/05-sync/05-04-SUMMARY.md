# 05-04 Summary — Phase-05 debt rows: hub.html under the gates, dev db, SOURCE.txt, store writeback

**Status:** COMPLETE (2026-11-25; Tasks 1-2 landed in the prior session, Task 3 this session)
**Plan:** `.paul/phases/05-sync/05-04-PLAN.md`

## Task statuses

| Task | Status | Evidence |
|------|--------|----------|
| 1 — hub.html under the gate set | done | 222 → 0 hardcodes; 8/8 receipts hub.html + index.html, served |
| 2 — dev db (DEBT-031) + SOURCE.txt (DEBT-032) | done | fresh db migrated, old preserved; Option A proven through checkout→write |
| 3 — store writeback + decision entries | done | checker re-derived set applied; DECISIONS.md contiguous D-001..D-025 |

## Task 1 — hub.html (the section split)

Substitution job, not a redesign (ponytail). Landed as one pass rather than section-split: the token
layer (`:root` block mirroring the D4 shared.css scale) covered the whole page in one edit and the gate
run confirms each section, so a split would have cost three gate runs for the same evidence. Hardcode
count: **222 → 0**. Landmarks (`<header>`, `<main>`, h1), rail-button and layout-select aria-labels,
disabled/faint colours moved to `--dim` per the theme record. Behaviour preserved: charts load, engine
switch, timeframe buttons, sticky account slot at the bar's right edge (D-019) — checked by hand served.
`index.html` (10-line redirect stub) took the minimum the gates require.

**Gate-runner changes this task (R-10 note):** `gate_all.py` now measures design_lint **served** (L-020;
it imports design_lint's own COLLECT_JS/judge) and runs `--min-label 11` (spec: "the existing chart UI
keeps its sizes"). It is a checker — the mutation set is re-run if it is edited again.

**Receipt table (final state, served):** 8/8 gates receipted for `hub.html` and `index.html` in
`.paul/ds-receipts/current/`; the red-green proof for the set is the pre-change failing receipts kept
from 04-03 (hub.html 6/8 failing, hardcodes 222) — a gate that never failed red was shown red by that set.

## Task 2 — DEBT-031 and DEBT-032

- **DEBT-031:** `mv data/vtv.db data/vtv.db.2026-11-24` (preserved, never deleted); fresh dev db
  migrates and listens on 55432. No in-place migration was attempted: the drift predates the current
  migration set, so there is no migration to write — inventing one would encode a schema that never
  shipped. If the owner later wants the old data readable, that is a D-row decision, not a silent repair.
- **DEBT-032:** **Option A** — `ops/deploy.sh` already writes `hub/SOURCE.txt` after checkout on the
  deploy path; `ops/write_source.sh` is now the single AGPL-offer writer, deploy.sh calls it, README
  documents the post-checkout step. Proven through the failing order (L-027): write →
  `git checkout -f hub/SOURCE.txt` → write again → `grep -c 'commit: '` returns 1 on the file the route
  serves. **The row stays open**: its check greps `/app/vtradingview/repo/hub/SOURCE.txt`, host-only —
  the owner discharges it at the next deploy.

## Task 3 — store writeback, re-derived (as run, not as the plan predicted)

`debt.py --run-checks` re-run after Tasks 1-2 and the decision verification. **Re-derived discharge set
(differs from the plan's paragraph): `DEBT-027`, `DEBT-030`, `DEBT-031`** — each printed OK +
`discharged? set status = "done"`; all three now carry `status = "done"` with a dated discharge line in
their notes. **DEBT-032 stays open** (host-only check), as the plan requires. Final store state:
`overdue=0 scheduled=0 waiting=13 open=0 done=21 (reconciled bare at 05-03 UNIFY; the write-time count was stale) dropped=0`, zero discharge lines remaining.

Two store defects found and fixed while running this task:

1. **DEBT-031's old check (`bash scripts/dev.sh _svc db`) could never pass from debt.py**: `_svc db`
   `exec`s pgsqlite in the foreground (`scripts/dev.sh:82`), so the command never exits — the session's
   first `--run-checks` hung >5 min and was killed. The check now runs a self-terminating probe:
   start pgsqlite on spare port 55439 against `data/vtv.db`, wait 2 s, kill, grep the log for
   `TCP server listening on 127.0.0.1:55439`. Quoting note recorded in the row: debt.py's
   `subprocess(shell=True)` is cmd.exe, which strips an inner single-quote layer, so the binary path
   goes unquoted inside `bash -c "…"`. Controls run through debt.py's own executor: drifted db
   (vtv.db.2026-11-24) exits 1, fresh db exits 0.
2. **DECISIONS.md contiguity verified before writeback**: `D-001..D-025` contiguous, no duplicate ids,
   no decided entry edited. D-019/D-020/D-021 (DEBT-027) and D-024/D-025 were already full entries —
   transcribed by the crashed 04-03 session's successor; the checker's OK confirmed them.

## Decisions

No new D-rows. D-019/D-020/D-021/D-024/D-025 verified present in DECISIONS.md (already transcribed).

## Verification set (run bare, R-8)

| Check | Result |
|-------|--------|
| `python .paul/runs/gate_all.py --base http://127.0.0.1:8770 --pages hub.html,index.html --check-receipts` | 8/8 gates receipted (exit 0) |
| `cargo fmt --all --check` | clean (exit 0) |
| `cargo clippy --workspace --all-targets -- -D warnings` (bare) | clean after the Cargo.toml fix below |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features` | exit 0 (52s of tests) |
| `cargo test -p hub-core --features test-idp --test state` | 8 passed |
| `cargo test -p hub-core --features test-idp --test db_engines` | 9 passed, 2 ignored |
| `CHECK_0303=1 bash .paul/runs/check-0402.sh` | bare-exit **0**: 126 PASS, 0 FAIL, check-0303 **66/66** under the shipped CSP |

Two run-time findings folded in:

1. **`tests/state.rs` had no `[[test]]` entry in `crates/hub-core/Cargo.toml`** — bare
   `clippy --all-targets` built it without `test-idp` and failed with 6 unresolved-symbol errors.
   Fixed by adding the entry with `required-features = ["test-idp"]`, matching its siblings; the fix
   and the suite's green run are recorded in 05-01-SUMMARY.md.
2. **A stale `hub-server.exe` (PID 18936) was holding port 8771** from a cancelled earlier run —
   check-0402.sh's own guard caught it ("a stale server would make this run measure the wrong
   binary") and refused. Killed by PID; the run then measured the fresh test-idp build.

## Skill audit

| Expected | Invoked | Notes |
|----------|---------|-------|
| /design-tokens, /design-doctrine | yes (Task 1, prior session) | token layer from hub/theme.js + design-tokens.json; DEBT-014's missing token not invented |
| /modern-web-guidance | yes (Task 1, prior session) | searched per capability before editing |
| /a11y-audit, /design-taste-frontend, /huashu-design, /design-gate | yes (Task 1, prior session) | satisfied by the gate_all receipt set + served browser pass (gate-runner satisfaction note) |
| pstack plan skills | n/a | pstack_stage: plan; APPLY ran in-session |
| context-budget, caveman, ponytail | yes | loaded this session (Task 3) |

## Next-phase readiness

Wave 1 of 3 complete. Next: **05-02 (HTTP: `routes/state.rs`, contract table, the `{key}` template
gate-test fix, the `spawn_background` prune, /security-review before Task 1)**, then 05-03 (client).
All wave-1 work is uncommitted on `arch/axum-tauri`.
