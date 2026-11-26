# PAUL Handoff

**Date:** 2026-11-26T16:10+07:00
**Status:** paused (phase 08 plans approved, APPLY not started)

---

## READ THIS FIRST

You have no prior context. This document tells you everything.

**Project:** vtradingview — free multi-feed TradingView-style charts in browser and desktop, each
user's work in their Google account.
**Core value:** charts + synced state + desktop + per-user MT5 bridges (read-only market data only).

---

## Current State

**Version:** 0.3.0 (milestone v0.3 Bridge platform, created this session)
**Phase:** 08 of 2 v0.3 phases — Rust bridge and mise distribution
**Plan:** 08-01 approved (option 1), APPLY not started

**Loop Position:**
```
PLAN ──▶ APPLY ──▶ UNIFY
  ✓        ○        ○     [plans 08-01..08-03 approved, await APPLY 08-01]
```

---

## What Was Done

- Milestone v0.3 Bridge platform created (ROADMAP phases 08/09, dirs, STATE, paul.toml, ledger,
  MILESTONE-CONTEXT.md consumed and deleted).
- **RustMT5 verified and rejected** (cloud gateway mt5.mrpc.pro hosts the terminals; owner keeps
  the current deployment). **mcp-proxy (joshrotenberg) verified and rejected** (many-to-one Tower
  gateway; no documented upstream Mcp-Session-Id; hand-rolled ~150-line client wins).
- **Verified in code:** bridge/mt5_bridge.py is a pure MCP proxy (5 tools/call names, no SDK).
  PROJECT.md constraint amended accordingly.
- Phase 08 planned with architect (arena): candidate B synthesized as the base (candidate A was
  stopped by the owner before completing — recorded sanctioned deviation, single candidate).
- Wrote `.paul/phases/08-rust-bridge/08-ARCHITECTURE.md` (the contract) + 3 plans
  (08-01 crate+mcp+term, 08-02 live+http+main, 08-03 contract suite+distribution+retirement).
- Plans approved by the owner (option 1) right before this pause.
- Debt store: 0 overdue, 15 waiting. DEBT-040/041/042 re-pointed phase:08 -> phase:10; DEBT-043's
  `owner-wants-permanent-bridge` event FIRED (discharge moves into phase 09).

---

## What's In Progress

- Nothing mid-flight: APPLY 08-01 has not begun. STATE.md still shows the milestone-creation
  position; its Session Continuity block (updated by this pause) is the truth.
- The STATE/ROADMAP/paul.toml/ledger "plan created" sync listed in the plan workflow was not yet
  written when the pause landed — resume should do it as step one (exact content below).

---

## What's Next

**Immediate:** finish the plan-created state sync (STATE.md Current Position -> "Plan: 08-01
created, approved; APPLY next", Loop PLAN ✓, ROADMAP phase 08 status "Planning"/"In progress",
paul.toml loop.position=PLAN + loop.plan="08-01" + loop.plan_path, ledger entry action="plan"
phase=8 plan="08-01"), then run `/paul:apply` for
`.paul/phases/08-rust-bridge/08-01-PLAN.md`.

**After that:** 08-02 (wave 1, depends 08-01), then 08-03 (wave 2: contract suite, mutations,
release windows leg, mise.mt5.toml, host-install revision, Python retirement). Phase 09 (manager
app, OTP pairing) follows as its own plan set.

---

## Key Files

| File | Purpose |
|------|---------|
| `.paul/phases/08-rust-bridge/08-ARCHITECTURE.md` | the phase contract: module map, parity contract (7 rules), test architecture, distribution |
| `.paul/phases/08-rust-bridge/08-01-PLAN.md` | approved: crate skeleton, sealed mcp.rs, term.rs |
| `.paul/phases/08-rust-bridge/08-02-PLAN.md` | approved: PollerSet, http router, main shell |
| `.paul/phases/08-rust-bridge/08-03-PLAN.md` | approved: fake MCP terminal, contract suite, 6 mutations, release/mise/host-install, retirement |
| `bridge/mt5_bridge.py` | the Python oracle (321 lines; retired only in 08-03 after parity) |
| `.paul/DEBT.toml` | 0 overdue; DEBT-043 fires with phase 09 |
| `.paul/STATE.md` | live position (Session Continuity updated by this pause) |

Uncommitted this session: `.paul/ROADMAP.md`, `.paul/STATE.md`, `.paul/PROJECT.md`,
`.paul/paul.toml`, `.paul/ledger.toml`, `.paul/DEBT.toml`, `.paul/HANDOFF-*` (this file),
`.paul/phases/08-rust-bridge/*` (4 files), `docs/mt5-bridges.md` unchanged. Project rule: commit
only when the owner asks (auto_commit: false).

---

## Resume Instructions

1. Read `.paul/STATE.md` Session Continuity (this handoff is the Resume pointer)
2. Read `.paul/phases/08-rust-bridge/08-ARCHITECTURE.md` before any 08-0N work (it is the contract)
3. Do the plan-created state sync listed under What's Next, then `/paul:apply` 08-01

Gotchas the next session must know:
- D-032: the 4-route JSON surface is FROZEN (hub-core bridges/client.rs is the consumer).
- D-005: only workspace-pinned deps; subtle + dotenvy are the two new crate-scoped additions.
- Root `cargo test --workspace` must stay tauri/webkit-free (pure Rust crate keeps it true).
- L-003/L-009/L-024/L-026 apply to every check the suite adds (mutations at observable guards,
  landing greps, negative controls, --list counts). R-8: quote exit statuses bare.
- The MT5 terminal exists only on the owner's host (103.93.92.204, Windows). The fake MCP terminal
  is the dev/CI oracle; the real host is phase 09's checkpoint.

---

*Handoff created: 2026-11-26T16:10+07:00*
