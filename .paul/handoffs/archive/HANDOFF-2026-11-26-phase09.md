# PAUL Handoff

**Date:** 2026-11-26T19:10+07:00
**Status:** session-reset (phase): phase 08 done

---

## READ THIS FIRST

You have no prior context. This document tells you everything.

**Project:** vtradingview — free multi-feed TradingView-style charts in browser and desktop, each
user's work in their Google account.

---

## Current State

**Version:** 0.3.0 (milestone v0.3 Bridge platform, 1 of 2 phases complete)
**Phase:** 09 of 2 v0.3 phases — Bridge manager app
**Plan:** none; ready to plan

**Loop Position:**
```
PLAN ──▶ APPLY ──▶ UNIFY
  ○        ○        ○     [Phase 09: ready to plan]
```

---

## What Was Done (phase 08, committed)

- Phase commit `7317378 feat(08): rust MT5 bridge, mise distribution, python bridge retired`
  (30 files), then `a554d57` (transition sync) and `029c370` (STATE git state).
- `crates/mt5-bridge` (root workspace member, tauri-free): sealed MCP client, domain core,
  PollerSet, four-route axum surface, shell. 40 crate tests, workspace 200 passed.
- Parity proven: the 08-03 contract suite diffed the Rust bridge against the retired Python
  oracle — **33/33 cases identical, plain-JSON and SSE passes, 0 differences**.
- Six mutations red-then-green (offset-round, session-reopen, key-fields, limit-clamp,
  auth-exact, wait-deadline) via `.paul/runs/mutate-mt5c.sh`.
- Distribution: release.yml five-leg matrix, `mise.mt5.toml`, `bridge/host-install.ps1`
  rewritten; `bridge/mt5_bridge.py` deleted; docs name only the binary.
- Debt gate `phase-08-exit` clear; PROJECT.md evolved (v0.3.0, D-035/D-036 added).

---

## What's Next

**The phase reflect (pstack) runs FIRST in this fresh session** — that is why this reset
happened (config: `reflect: pstack`, `session_reset: plan`). It reads every SUMMARY in
`.paul/phases/08-rust-bridge/`, then `rsi`. Write the `reflect:`/`rsi:` lines into
`08-03-SUMMARY.md` (append under "Reflect and RSI", replacing the "pending" note).

**Then:** `/paul:plan` for phase 09 — the Tauri manager app on the MT5 VPS (install, OTP
pairing from Settings, control, status). ROADMAP's phase 09 block and
`.paul/phases/09-bridge-manager/` exist. The phase-08 deferred items are phase 09's
checkpoints: stale-session behavior on the real terminal, `time_ms` ms-digit confirmation,
ONLOGON-vs-ONSTART, the mise leg's real install on the host (103.93.92.204, Windows).

---

## Key Files

| File | Purpose |
|------|---------|
| `.paul/phases/08-rust-bridge/08-{01,02,03}-SUMMARY.md` | the reflect's inputs |
| `.paul/phases/08-rust-bridge/08-ARCHITECTURE.md` | the phase 08 contract (kept for reference) |
| `.paul/STATE.md` | live position (Session Continuity updated by this pause) |
| `.paul/LEARNINGS.md` | where reflect findings land |
| `.paul/runs/check-mt5c.sh`, `.paul/runs/mutate-mt5c.sh` | the standing parity + mutation suites |

Uncommitted: only files this pause writes (handoff, marker). Project rule: commit only when the
owner asks (auto_commit: false).

---

## Gotchas the next session must know

- D-032: the 4-route JSON surface is FROZEN (hub-core bridges/client.rs is the consumer).
- D-035: every wire float goes through `py_str` (Rust `{}` is not Python `str(float)`).
- D-036: domain functions take `&M: Terminal`; in-memory fakes under test (L-016).
- D-005: only workspace-pinned deps; the crate-scoped set is subtle + dotenvy + axum + tower.
- Root `cargo test --workspace` must stay tauri-free (0 tauri/webkit/wry lines).
- L-003/L-009/L-016/L-026/R-8 govern every check (mutations at observable guards, landing greps,
  no sockets in domain tests, `--list` pins, bare exit statuses).
- The real MT5 host (103.93.92.204, Windows, Exness-MT5Real28) is phase 09's checkpoint target.
- CARL GLOBAL rules: justrust for cargo, ygrep/agentgrep first for searches.
- Owner decision stands: do NOT deploy; the live site stays v0.1.1.

---

*Handoff created: 2026-11-26T19:10+07:00 by the phase-08 transition (session-reset mode)*
