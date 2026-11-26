# 07-01 SUMMARY: bridge core (migration, token crypto, SSRF guard, BridgeClient)

**Applied:** 2026-11-26 (approved plans 07-01 + 07-02, applied in one wave, sequential in-session). **Status:** UNIFIED 2026-11-26.
**Plan:** `.paul/phases/07-bridges/07-01-PLAN.md` · **Contract:** `.paul/phases/07-bridges/07-ARCHITECTURE.md`

## Tasks
| # | Task | Status |
|---|------|--------|
| 1 | Migration 0003 mt5_bridges (uuid ids, timestamptz, FK cascade) + store B1..B5 owner-scoped, shape test | done |
| 2 | Token crypto: XChaCha20-Poly1305, nonce || ct, HMAC-SHA256 domain-separated key from VTV_SECRET_KEY | done |
| 3 | SSRF guard: refused-reason table (9 classes + mapped forms), vet_syntax, vet_url with pinned Vetted | done |
| 4 | BridgeClient: pinned connect (resolve_to_addrs), redirects off, 10 s / 25 s wait, 5 MB cap, per-bridge Semaphore(2), the seven BridgeError causes | done |

## Acceptance criteria
| AC | Result |
|---|---|
| AC-1 migration on both engines | green (pgsqlite 1/1; Postgres SKIPPED, VTV_TEST_PG_URL unset on this box: the claim cites the skip, L-039) |
| AC-2 crypto roundtrip, tamper, wrong-key, short blob, empty token, per-record nonces | green |
| AC-3 every refused class (17-case table, auth-off lifts loopback only) | green |
| AC-4 redirect refused, 5 MB cap (early content-length + streamed accumulate), 10 s timeout, wait's 25 s, semaphore of 2 proven by the fake's peak-connection count | green |
| AC-5 m1 (CGNAT row disabled): the named table test FAILED red, landed-grep 1, restore + touch, 4/4 green | proven (L-003/L-009/L-013) |

## Verification
- `cargo test -p hub-core --lib`: **133 passed, 0 failed**, exit 0 (bare)
- `cargo test -p hub-core --features test-idp --test bridges`: 1 passed (pgsqlite); `--test db_engines`: 9 passed, **2 ignored** (Postgres SKIPPED, VTV_TEST_PG_URL unset)
- `cargo fmt --all --check` clean, exit 0 bare (R-8); `cargo clippy -p hub-core --all-targets --all-features`: **0 warnings** (the test-idp way included)
- m1 mutation red-then-green as AC-5 records

## Deviations (each with its reason)
1. **DDL types fixed forward**: the plan's DDL said INTEGER PK, BIGINT user_id, BIGINT times; the repo schema is uuid PK, uuid user_id REFERENCES users(id) ON DELETE CASCADE, timestamptz (the 0001/0002 convention). The migration, store and 07-03's want keys (`mt5:<uuid>`) follow the repo convention.
2. **Transport trait dropped**: the plan's client test seam became real-loopback fakes over hand-built `Vetted` values; the https rule is the guard's contract and is tested there. One fewer abstraction, same coverage.
3. **reqwest gained the `stream` feature** (the 5 MB cap streams the body); a feature flag on an existing dependency, not a new one.
4. **`bars` takes a `BarsQuery` struct** (clippy too_many_arguments at 8/7).
5. **The guard implements D-033's scheme rule**: plain `http://` is accepted with auth off only when every resolved address is loopback (the local bridge); auth-on stays https-only.
6. **tests/db_engines.rs updated with the migration**: versions [1,2,3], `mt5_bridges` joins the down-list, the failing-third case moved to version 4 because the real 0003 applies at `Db::open`.

## Deferred / debt
None new. DEBT-038 (first push) and DEBT-039 (owner real-bridge uat, blocking phase-07-exit) wait by design; the wave's discharges (DEBT-032/035) are 07-02's.

## Security review (the phase override, whole wave-1 diff)
No Critical, no High. The method: adapted fast-scan greps, every candidate confirmed by reading.
- The guard vets **addresses**, not host text: octal/hex host-text evasions (`0177.0.0.1`, `0x7f000001`) resolve to private addresses and are refused at the resolve-side table; DNS rebinding is dead because the client connects to the pinned `Vetted` addresses, and a re-vet with different addresses builds a new pinned client.
- Token material never logs: the bridge's `log()` names the variable, never the value; the client sends the token as a header and carries no token in any error text it composes.
- AEAD (XChaCha20-Poly1305) with OS nonces (getrandom) and an HMAC-domain-separated key; rows hold `nonce || ct` only.
- One tracked note: a hostile bridge can echo the Authorization value into its error text, which lands in `last_error` (owner-visible only) and, from 07-03, in the logbus; 07-03's planned logbus redaction covers the logbus half.

## Skill audit
| Skill | Loaded? | Note |
|---|---|---|
| context-budget, caveman, ponytail | yes (session) | AGENTS.md rule |
| /architect | yes (plan) | 07-ARCHITECTURE.md is the sketch |
| /security-review | yes (apply) | whole wave-1 diff; findings above |
| /paul-security | not invoked | the phase override's whole-diff review subsumes per-task guidance |
| pstack | pstack_stage: plan | APPLY ran in-session (paul-apply Step 2); no poteto-mode routing |
| parallel_agents: true | degraded to sequential | ZCode has no worktree isolation (the archived handoff's gotcha 2) |

## Reflect and RSI
Deferred to phase close (reflect: pstack; this is not the phase's last plan). This unify's close-out writes: learnings +2 added (L-045 fake's dual-role counter, L-046 read the repo's migration conventions before writing plan DDL), 5 bumped (L-009, L-013, L-023, L-039, L-043); debt +0 added, +2 discharged (DEBT-032/035, 07-02's); rules +0, decisions +0 (D-032/D-033 landed at plan approval). Re-runnable gates re-run at unify: 133/133 lib tests exit 0, check-0702 all green, check-032 prints 0.

## UNIFIED ≠ LANDED
The wave-1 diff (crates/hub-core/src/bridges/, migrations 0003, tests/bridges.rs, bridge/mt5_bridge.py, docs/mt5-bridges.md, the .paul state files) is uncommitted (auto_commit: false); the phase commit lands it, as phases 05/06 did. Artifact rows for the commit are filed at the phase close, per the DEBT-024/038 pattern.
