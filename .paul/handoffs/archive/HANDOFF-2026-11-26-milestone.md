# PAUL Handoff

**Date:** 2026-11-26T14:10+07:00
**Status:** session-reset (phase): milestone v0.2 Accounts everywhere done

---

## READ THIS FIRST

You have no prior context. Durable state lives in `.paul/STATE.md`, `.paul/ROADMAP.md`,
`.paul/MILESTONES.md` (v0.2 entry), `.paul/milestones/v0.2-ROADMAP.md` (archive),
`.paul/DECISIONS.md`, `.paul/DEBT.toml`, `.paul/LEARNINGS.md`. This file carries only what lives
nowhere else.

**Project:** vtradingview (Axum hub + Google sign-in + Tauri shell), branch `arch/axum-tauri`
**Core value:** Free multi-feed TradingView-style charts in a browser or desktop app, with each user's
layouts and scripts kept in their Google account.

---

## Current State

**Milestone:** v0.2 Accounts everywhere — COMPLETE 2026-11-26 (3 phases, 12 plans). Tag `v0.2.0`
created, not pushed.
**Phase:** none active. Phase 07 (Per-user MT5 bridges) closed 2026-11-26T13:10+07:00.
**Loop:** idle, all three markers clear.
**Tree:** the milestone's own work is committed and pushed (`63cd4e4`, `5d9c509`, `d235b03`,
`e3f4174`); CI on that push was green (DEBT-038). **Uncommitted at this pause** (`auto_commit: false`,
so nothing was committed): `.paul/STATE.md`, `.paul/SPECIAL-FLOWS.md` (R-13 + amendment row),
`.paul/HANDOFF-2026-11-26-milestone.md`. `.paul/session-reset.json` is gitignored by design.

The ONE next action is a choice the user owns, so the resumed session presents it and stops:

- `/paul:discuss-milestone` — define the next milestone's scope
- `/paul:milestone` — create it directly from ROADMAP.md's next phase
- review / stop

---

## What Was Done (this session)

Nothing new was built. This session found and fixed a framework gap:

- **`complete-milestone.md` ignored `preferences.session_reset`.** UNIFY (`unify-phase.md`
  `route_based_on_completion`) and phase transition (`transition-phase.md` `route_next`) both honor it;
  milestone close did not. Result: v0.2 closed at 13:40 with no handoff, no marker, no fresh-session
  prompt — the exact cost `session_reset: plan` exists to avoid.
- Fixed in two places:
  - `~/.agents/paul-framework/workflows/complete-milestone.md` `<step name="offer_next">` — reads the
    key first; `plan`/`phase` prints the celebration block without its choice lines, then runs
    `pause-work.md` in session-reset mode (boundary `phase`, closed `milestone {version}`, next = the
    user-owned milestone choice) and stops the turn.
  - `~/.agents/skills/paul-complete-milestone/SKILL.md` — step 4.5 states the same, plus a success
    criterion checkbox.
- Verified the harness side: ZCode `~/.zcode/cli/config.json` `hooks.events.SessionStart` already runs
  `session_reset_hook.py` (and Claude Code `~/.claude/settings.json` at `startup|clear`), so a fresh
  session in this directory auto-continues into paul-resume.

---

## Why it mattered (the mechanism, for the next reader)

`session_reset: plan` is documented as "every plan's UNIFY **and every phase transition**". A milestone
close reached directly — phases already closed in earlier sessions — is a boundary strictly larger than
a phase close, and it was the one path with no reset. The archive is durable; the reasoning that produced
it (what was deliberately left open, which rows are waiting on the owner, the do-not-deploy standing
decision) was only in the transcript.

---

## What's Next

**Immediate:** the milestone choice above. Standing work while deciding lives in `.paul/DEBT.toml`:

- Owner rows: Q2 (data terms), Q3 (trademark), Q9 (privacy text), DEBT-020/026, DEBT-043 (permanent
  tunnel + `bridge\bridge-registration.txt` credential handoff).
- Agent rows: DEBT-040 (`gate_all.py` must refuse a server whose `/api/health` is not the fresh-ephemeral
  identity, and stamp identity into receipts), DEBT-041 (`debt.py` creation-time check validation —
  negative control + DUE-NOW phrase collision), DEBT-042 (phase-07 security dispositions: re-arm logbus
  redaction at boot, cap per-user bridge rows, close guard's adjacent v4/v6 classes).

**Standing decision still in force:** do NOT deploy; the live site stays v0.1.1.

---

## Key Files

| File | Purpose |
|------|---------|
| `.paul/STATE.md` | Live project state, cleared for the next milestone |
| `.paul/MILESTONES.md` | v0.1 + v0.2 entries with stats |
| `.paul/milestones/v0.2-ROADMAP.md` | v0.2 archive |
| `.paul/DEBT.toml` | 0 overdue, 16 waiting, 27 done |

---

## Resume Instructions

1. Load CARL GLOBAL rules (justrust for cargo; ygrep/agentgrep for searches) and `context-budget`,
   `caveman`, `ponytail`.
2. Run `/paul:resume`. The marker makes it auto-continue: it restores context, runs the debt gate,
   deletes `.paul/session-reset.json`, and presents the milestone choice.
3. That choice is the user's: present the options, do not pick one.

---

*Handoff created: 2026-11-26T14:10+07:00*
