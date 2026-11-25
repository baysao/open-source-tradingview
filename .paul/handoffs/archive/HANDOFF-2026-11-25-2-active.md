# PAUL Handoff

**Date:** 2026-11-25T16:32+07:00
**Status:** paused — phase 05 apply, waves 1-2 of 3 complete, 05-03 next

---

## READ THIS FIRST

You have no prior context. This document tells you everything.

**Project:** vtradingview (free multi-feed TradingView-style charts in a browser/desktop; layouts and
scripts live in each user's Google account).
**Core value:** Milestone v0.1 shipped. v0.2 "Accounts everywhere" phase 05 (Synced state) is applying.

---

## Current State

**Phase:** 5 of 7 — Synced state
**Plan:** 05-02 COMPLETE (SUMMARY written, uncommitted); 05-03 next (client)
**Loop:**
```
PLAN ──▶ APPLY ──▶ UNIFY
  ✓        ●        ○     [05-01, 05-04, 05-02 done; 05-03 next]
```

---

## What Was Done (this session)

- **05-02 applied end to end**: four `/api/me/state` handlers (`crates/hub-core/src/routes/state.rs`),
  four `{key}` specs in `routes/table.rs:65-80`, the gate test's template fix
  (`tests/gate.rs:24-42` `sample_path()` + `!= 404`), the daily backup prune beside the hourly sweep
  (`src/lib.rs:376-394`), and 9 HTTP contract tests (`tests/state_routes.rs`).
- **Both plan-named mutations were non-biting as written** and replaced (see SUMMARY "Mutation run"):
  the gate mutation belongs at the *mount* (`axum::Json` extractor runs before the handler body), the
  prune mutation belongs on *S21* (`AND key >= $2` → `OR`, left:3/right:1). Both failed the suite,
  restored, re-proven after the checker edits (R-10).
- **Security review** (loaded before Task 1): `get_one`/`delete` reordered to answer 401 before any
  key check (AC-6); signed-out test now asserts `COUNT(*) FROM user_state` == 0 after.
- **Verification green on both engines**: state_routes 9, state 8, gate 7, admin 1, workspace 192,
  fmt + clippy clean. Postgres half needed a throwaway PG 18.7 cluster (`%TEMP%/vtv-pg18`, port
  55433, `VTV_TEST_PG_URL`), stopped after the run.
- **Debt rows**: DEBT-033 (256-key cap is corrective, check greps `D-026` in DECISIONS.md — the first
  check passed day-one and was rewritten), DEBT-034 (no docs page for the contract).
- **Artifacts**: `05-02-SUMMARY.md` (contract table 05-03 codes against), STATE/paul.toml/ledger/
  DEBT updated, session decision row added (401-before-key-check).

---

## What's In Progress

- Nothing mid-task. All wave-1 + wave-2 work is **uncommitted** on `arch/axum-tauri`
  (`git status` shows the full surface: state.rs, state_routes.rs, table.rs, mod.rs, lib.rs,
  gate.rs, Cargo.toml `[[test]]` entries, the .paul files, three SUMMARYs).

---

## What's Next

**Immediate:** `/paul:apply 05-03` — the client plan (sync.js against 05-02's contract table:
IndexedDB mirror, per-key debounce, merge on 409, backup key naming, CSP/index.html wire-up).
**After that:** UNIFY for phase 05 (all four plans), then the phase commit.

---

## Key Files

| File | Purpose |
|------|---------|
| `.paul/phases/05-sync/05-02-SUMMARY.md` | The contract table + mutation records + security findings |
| `.paul/phases/05-sync/05-03-PLAN.md` | Next plan (client) |
| `.paul/STATE.md` | Live project state (updated this session) |
| `.paul/DEBT.toml` | DEBT-033/034 added; overdue=0 |
| `.paul/handoffs/archive/` | Two earlier handoffs from today, archived |

---

## Resume Instructions

1. Read `.paul/STATE.md` for latest position
2. Check loop position (APPLY, 05-03 next)
3. Run `/paul:resume`

**Gotchas that live nowhere else:**
- `engines!` tests SKIP Postgres unless `VTV_TEST_PG_URL` is set; this box has no running cluster.
- `justrust` not bare `cargo`; `ygrep` first for searches (CARL GLOBAL/BUILD rules).
- carl-mcp: use the `carl_v2_*` family; `carl_get_manifest`/`carl_get_domain_rules` fail on this
  host. The owner is debugging carl-mcp and wants carl calls announced in text.
- `%2F` is the only way a `/`-containing key reaches `{key}` (literal `/` is a second segment).

---

*Handoff created: 2026-11-25T16:32+07:00*
