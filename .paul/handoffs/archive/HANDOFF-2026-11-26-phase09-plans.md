# PAUL Handoff

**Date:** 2026-11-26T19:22+07:00
**Status:** paused (context limit approaching) — plans 09-01..09-03 written, awaiting approval

---

## READ THIS FIRST

You have no prior context. This document tells you everything.

**Project:** vtradingview — free multi-feed TradingView-style charts in browser and desktop, each
user's work in their Google account.
**Core value:** free multi-feed TradingView-style charts, layouts and scripts in the user's account.

---

## Current State

**Version:** 0.3.0 (milestone v0.3 Bridge platform, 1 of 2 phases complete)
**Phase:** 09 of 2 v0.3 phases — Bridge manager app
**Plan:** 09-01/09-02/09-03 created, awaiting owner approval

**Loop Position:**
```
PLAN ──▶ APPLY ──▶ UNIFY
  ✓        ○        ○     [plans created, awaiting approval]
```

---

## What Was Done (this session)

- **Phase-08 reflect (pstack) closed.** 3 reviewer lenses over the extracted transcript
  (`.paul/runs/reflect-08/`), parent-verified by live probe before accepting. Consumed the
  session-reset marker and HANDOFF-2026-11-26-phase09.md (archived to `.paul/handoffs/archive/`).
- **One live defect found and fixed:** `/symbols` key order — the built binary emitted
  `{"desc","digits","name","path"}` (BTreeMap) against contract rule 6's `{"name",...}`. Hidden by
  the differ's `sort_keys=True` AND by dev-only `preserve_order` feature unification. Fixed:
  feature moved to the normal dependency in `crates/mt5-bridge/Cargo.toml` (dev override dropped),
  differ un-sorted + `assert total == 33` pinned. Re-probed green; ALL gates re-run after the edit
  (R-10): fmt/clippy 0, 40 crate tests, 200 workspace tests, 33/33 parity × 2 passes, 6 mutations
  red/green (exit 0).
- **Learnings written:** L-051..L-054 (differ key order, wrong-signed mutation + chained restore,
  port-must-not-harden-crash-paths, untracked accept evidence), L-023 hit 6, L-026 hit 3.
- **DEBT-044 filed** (parity suite untracked; trigger phase:09) — discharged IN 09-01 Task 5.
- **Phase 09 planned** (3-plan split confirmed by owner): `09-ARCHITECTURE.md` + 09-01 (hub
  pairing surface), 09-02 (manager Tauri app), 09-03 (install/host/checkpoints/DEBT-043).
- STATE.md deduped + Session Continuity rewritten; ROADMAP phase 09 → Planning.

---

## What's In Progress / Pending Owner

- **Plan approval** — the three plans are unreviewed.
- **Two instruction edits await the owner's yes** (LEARNINGS policy: never auto-apply):
  1. R-8 tighten (SPECIAL-FLOWS.md): bare `; echo exit=$?` only valid with nothing piped — the
     phase-08 session printed `clippy -p exit=0` beside 10 compile errors via the licensed shape.
  2. context-budget exemption line: judged gate commands (must quote a bare exit) are exempt from
     the no-single-call rule; batch reconnaissance, never judged runs.
  (Plus the older pending one: tune design-taste-frontend's look-pass step.)
- ~13 uncommitted files (reflect outputs + plans); auto_commit: false.

---

## What's Next

**Immediate:** owner reviews + approves the plans → `/paul:apply .paul/phases/09-bridge-manager/09-01-PLAN.md`
**After that:** 09-02 (depends 09-01), then 09-03 (host checkpoints on 103.93.92.204, DEBT-043
discharge, the phase's human-verify accept).

---

## Key Files

| File | Purpose |
|------|---------|
| `.paul/phases/09-bridge-manager/09-ARCHITECTURE.md` | the phase-09 sketch + 7 decisions |
| `.paul/phases/09-bridge-manager/09-0{1,2,3}-PLAN.md` | the three plans (approval targets) |
| `.paul/STATE.md` | live position (Session Continuity current) |
| `.paul/LEARNINGS.md` | L-051..054 + the two hit bumps |
| `crates/mt5-bridge/Cargo.toml` | the preserve_order fix (committed? check git status) |
| `.paul/runs/diff_bridge_contract.py` | un-sorted + total==33 pin |
| `.paul/DEBT.toml` | DEBT-043 (phase 09), DEBT-044 (phase 09), 041/042/040 (phase 10) |

Debt: **0 overdue**, 16 waiting, 28 done. Nothing scheduled. DEBT-043 + DEBT-044 are phase-09's
rows (both covered by the plans; silent omission would be a plan defect — they are not omitted).

---

## Gotchas the next session must know

- D-032: the 4-route bridge JSON surface is FROZEN (hub-core bridges/client.rs consumes it).
- D-030: `crates/mt5-manager` (09-02) must be its own Cargo workspace — never a root member; the
  root `cargo test --workspace` must stay tauri-free (0 tauri/webkit/wry lines, CI-asserted).
- D-005: workspace pins only; 09-01 adds NO new dependency (sha256/getrandom/base64 are in).
- R-8 + L-023: quote exit statuses bare; a pipe's `$?` is its last stage's. R-10: editing a
  checker invalidates its mutation results (the mt5c suite files must not be edited in 09-01).
- L-024: a DEBT row's discharge pattern lives in STATE.md's record line only — never in a DUE NOW
  text (DEBT-043's check anchors on "DEBT-043 discharged: named tunnel live").
- The pairing design: codes are 10-digit rejection-sampled (never `x % 10^10`), 10-min TTL,
  burned before check (S12/S13 semantics); revoke is a second per-bridge secret, not the code.
- Owner decision stands: do NOT deploy; the live site stays v0.1.1.
- CARL GLOBAL rules: justrust for cargo, ygrep/agentgrep first for searches. English always
  (owner preference, stated 2026-11-26).

---

## Resume Instructions

1. Run `/paul-resume` (it reads STATE.md, finds this handoff, suggests the ONE next action).
2. Approval flow: review 09-01 first; apply proceeds 09-01 → 09-02 → 09-03.

---

*Handoff created: 2026-11-26T19:22+07:00 by /paul-pause (context limit)*
