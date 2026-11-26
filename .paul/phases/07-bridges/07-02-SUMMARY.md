# 07-02 SUMMARY: Python bridge hardening, user docs, the phase's debt discharge

**Applied:** 2026-11-26, same wave as 07-01. **Status:** UNIFIED 2026-11-26.
**Plan:** `.paul/phases/07-bridges/07-02-PLAN.md`

## Reflect and RSI
Deferred to phase close (reflect: pstack; not the phase's last plan). Shares the wave's close-out writes (07-01-SUMMARY records the delta): learnings +2, 5 bumped; debt +2 discharged (this plan's DEBT-032/035). Gates re-run at unify: 133/133 lib tests, check-0702 all green, check-032 prints 0.

## UNIFIED ≠ LANDED
The wave-1 diff is uncommitted (auto_commit: false); the phase commit lands it, per the phase-05/06 pattern. Artifact rows at the phase close.

## Tasks
| # | Task | Status |
|---|------|--------|
| 1 | `bridge/mt5_bridge.py`: BRIDGE_TOKEN required (refuses to start), constant-time bearer check on every route, BRIDGE_BIND option; route shapes unchanged | done |
| 2 | `docs/mt5-bridges.md`: env vars, both publishers, registration, the exact-error table, local dev, security notes | done |
| 3 | DEBT-032 discharge | done |
| 4 | DEBT-035 discharge | done |

## Acceptance criteria
| AC | Result |
|---|---|
| AC-1 refuse-to-start + 401 bare + 401 wrong bearer + right-bearer clears auth | check-0702.py: 5/5 PASS, exit 0 |
| AC-2 BRIDGE_BIND default loopback; shapes unchanged (right-auth /health 502 = the request reached the terminal layer, never 401) | proven by the same self-test |
| AC-3 docs/mt5-bridges.md: env vars, both publishers, registration, the exact-error table, local dev, security notes; house rules clean (grep emoji/em-dash = 0) | green |
| AC-4 DEBT-032: the order fix was already in place (deploy.sh:24 to :52; README:138 to :140); the check rewritten repo-runnable (.paul/runs/check-032.sh prints 0); negative control (swapped order exits 1); row done | green |
| AC-5 DEBT-035: gate_all.py docstring names the auth-off base (grep = 1; 0 before the edit = the negative control); row done | green |

## Verification
- `python -I .paul/runs/check-0702.py` exit 0; `python -m py_compile bridge/mt5_bridge.py` ok
- `bash .paul/runs/check-032.sh` prints 0, exit 0; negative control exits 1
- `debt.py --run-checks`: overdue 0, **waiting 13** (14 minus the two discharges, with DEBT-039 filed at plan time), done 26

## Deviations
1. **The deploy.sh/README order fix was already present** (corrected in earlier phase work, both files already cite DEBT-032); the task's remaining half was the repo-runnable check, the negative control and the row edit. Recorded honestly rather than re-fixing working code.
2. **check-032.sh asserts both files** (deploy.sh + the README sequence): the row's own note names both fix options, so the check covers both ways a checkout can revert the AGPL offer.

## Skill audit
Same session rows as 07-01 (context-budget/caveman/ponytail loaded; architect at plan; /security-review over the whole wave-1 diff; pstack_stage: plan, in-session; parallel_agents degraded to sequential).
