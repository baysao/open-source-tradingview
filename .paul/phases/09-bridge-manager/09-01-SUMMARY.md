---
phase: 09-bridge-manager
plan: 01
type: Summary
about: "vtradingview"
---

# 09-01 SUMMARY: the OTP pairing surface

**Status:** APPLY complete, all ACs pass, all gates green.
**Executed:** 2026-11-26. Sequential (D-034: `parallel_agents: false` here).

## What shipped

A hub-side pairing surface: Settings mints a ten-digit single-use code; the manager app (09-02) spends
it at `POST /api/me/mt5/pair` and registers a bridge through register's exact flow; a per-bridge
revoke secret deletes exactly that bridge at `POST /api/me/mt5/paired/{id}/revoke`.

| Route | Auth | Answer |
|---|---|---|
| `POST /api/me/mt5/pairing-code` | session cookie | 201 `{code, expires_at}` |
| `POST /api/me/mt5/pair` | one-time code in body | 201 `{id, label, url, symbol_count, connected, revoke_code}` |
| `POST /api/me/mt5/paired/{id}/revoke` | per-bridge secret in body | 204 |

Files: `crates/hub-core/migrations/0004_pairing.{up,down}.sql`, `src/db.rs`, `src/auth/ids.rs`,
`src/auth/store.rs`, `src/auth/mod.rs`, `src/bridges/store.rs`, `src/routes/pairing.rs` (new),
`src/routes/mt5.rs`, `src/routes/gate.rs`, `src/routes/mod.rs`, `src/routes/table.rs`,
`hub/settings.js`, `hub/settings.html`, `tests/pairing_routes.rs` (new), `tests/gate.rs`,
`tests/db_engines.rs`.

## AC results

| AC | Result | Evidence |
|---|---|---|
| AC-1 migration + store | pass | `cargo test --features test-idp --test db_engines`: 9 passed, 2 ignored; `up_up_down_up` pins versions `[1,2,3,4]` and both new tables dropped cleanly |
| AC-2 mint session-gated, bounded | pass | `the_mint_is_session_gated_and_repeats`: signed-out 401, `^\d{10}$`, two live codes; unit `a_pairing_code_is_ten_digits_zero_padded` |
| AC-3 pair burns + reuses register | pass | `pair_burns_the_code_and_reuses_register`, `pair_vets_the_url_before_any_network_call`; one code = one attempt |
| AC-4 revoke removes one bridge | pass | `revoke_deletes_exactly_one_bridge`, `revoke_never_touches_another_users_bridge` |
| AC-5 D-032 properties on the new surface | pass | 4 mutation failures (below), `the_origin_rule_still_guards_everything_but_the_two_secret_paths`, `only_the_two_secret_authenticated_paths_skip_the_origin_rule`, `the_sweep_removes_expired_pairing_codes` |
| AC-6 Settings shows the code | pass | 8/8 × 5 pages served; dialog open state measured by hand (Esc closes, code cleared, no covered focusable) |

## Deviations (each verified, none absorbed silently)

**1. The plan's AC-5 contradicts its AC-3 on the Origin rule — exemption added.** AC-5 wanted both
new endpoints to answer 401 signed-out "inside the gate area", but AC-3 requires `pair` to run
"without any session cookie", and the gate 403s every non-GET in `/api/me` that lacks
`Origin == PUBLIC_ORIGIN`. The manager app is a Rust process: it sends no Origin at all. As written,
`pair` and `revoke` were unreachable from their only real caller.

Fix: `routes/gate.rs` gained `is_secret_authenticated`, which exempts **exactly two paths** —
`/api/me/mt5/pair` and `/api/me/mt5/paired/{id}/revoke` — matched structurally (slice pattern, not
prefix), so the session-gated mint `/api/me/mt5/pairing-code` and every other `/api/me/*` path keep
the rule. This does not weaken D-010: D-010 stops a hostile page riding the user's cookie, and these
two paths never read the cookie — the body secret *is* the credential. Unit test
`only_the_two_secret_authenticated_paths_skip_the_origin_rule` asserts 10 negative cases so the
exemption cannot widen.

**2. `ALTER TABLE ... ADD COLUMN` is unproven on pgsqlite 0.0.22 — side table instead.** The plan put
`revoke_hash bytea` on `mt5_bridges` via ALTER. Every earlier migration proves `CREATE TABLE` on both
engines; none proves ALTER, and a pgsqlite failure would surface only at `Db::open` in production.
Fix: `mt5_bridge_secrets (bridge_id uuid PK REFERENCES mt5_bridges ON DELETE CASCADE, revoke_hash
bytea NOT NULL)`. This also left `NewBridge` and its three call sites untouched.

**Consequence worth naming:** `revoke` deletes by id with no owner in the statement (B9), because the
secret identifies the bridge. The owner check that makes this safe is `revoke_matches` (B7): the
delete is unreachable without that bridge's own secret. The 404-vs-401 split needs `exists` (B8), a
second single-table statement rather than a join (D-014).

**3. The plan's "rejection-sampling 4 bytes into [0, 10^10)" is impossible.** 2^32 ≈ 4.3e9 < 10^10.
Fix: sample `u64`, reject above `u64::MAX - (u64::MAX % 10^10)`. The bound is asserted in a
`const { assert!(...) }` block, so a regression to 4 bytes fails the **build**, not just a test.

**4. `tests/gate.rs` had to learn the exemption.** `origin_rules_over_every_hosted_non_get`
enumerates every hosted non-GET and asserted 403 for all of them. It now skips the two exempt paths
with an explicit `secret_auth` branch asserting not-403, so a *third* exempt path still fails the
enumeration.

**5. `tests/db_engines.rs` probe moved from version 4 to 5.** `FAILING_THIRD` used version 4 for its
deliberately-broken migration; 0004 now occupies that slot. Bumped to 5, and the version/tables pins
extended to include both new tables.

## Mutation evidence (L-003/L-009)

Two mutations of `crates/hub-core/src/bridges/guard.rs`, restored byte-identical each time
(`git status` clean afterwards):

| Mutation | Result |
|---|---|
| `if !https \&auth_enabled` → `\&\& false` (scheme rule off) | **2 tests red**: `pair_burns_the_code_and_reuses_register`, `pair_vets_the_url_before_any_network_call` |
| `return auth_enabled.then_some("loopback address")` → `return None` (both sites) | **3 tests red**: `the_mint_is_session_gated_and_repeats`, `pair_burns_the_code_and_reuses_register`, `a_code_minted_by_one_account_never_pairs_for_another` |
| `store::sweep_pairing_codes(...)` → `let codes: u64 = 0` (P4 off) | **1 test red**: `the_sweep_removes_expired_pairing_codes` (and only that one, which is the point) |

**A vacuous assertion this caught in my own first draft:** the loopback test originally asserted only
`status == 422`. With the guard disabled the request reaches a connection attempt that *also* answers
422, so the suite stayed **fully green under the mutation** — measured, not assumed. Every guard
assertion now pins the exact cause text (`"the bridge address was refused: loopback address"`,
`"the bridge url must be https"`), which is what makes the mutation fail. This is the L-003 placement
rule doing its job: the exemption path had to prove the guard runs on *it*, not nearby.

## Verification (bare exit statuses, R-8)

| Check | Exit |
|---|---|
| `cargo fmt --all --check` | 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 |
| `cargo clippy -p hub-core --all-targets --features test-idp -- -D warnings` | 0 |
| `cargo test --workspace` | 0 (0 tauri/webkit/wry lines) |
| `cargo test -p hub-core --features test-idp` | 0 (19 binaries ok; pairing_routes 10 passed) |
| `cargo test -p hub-core --features test-idp --test pairing_routes` | 0 (10 passed) |
| `cargo test -p hub-core --features test-idp --test gate` | 0 (7 passed) |
| `cargo test -p hub-core --features test-idp --test db_engines` | 0 (9 passed, 2 ignored) |
| `python .paul/runs/gate_all.py --pages hub.html,index.html,settings.html,privacy.html,notices.html` | 0 (8/8 × 5) |
| `python .paul/runs/gate_all.py --check-receipts` | 0 |
| `python ~/.agents/paul-framework/scripts/debt.py --run-checks` | 0 overdue, 0 scheduled, 15 waiting |

`bash .paul/runs/check-mt5c.sh`: **exit 1, not caused by this plan.** Both differ passes report
`DIFFER: 33/33 cases identical, 0 differences`; the failure is a separate inline assertion
(`check-mt5c.sh:110-113`) canonically diffing two sequential curls whose `measured_at` is a
wall-clock second (measured: `1795698259` vs `1795698260`; equal once `measured_at` is dropped).
`crates/mt5-bridge/` is untouched by this plan. Filed as **DEBT-045** with the reproduction and the
R-10 warning that editing the checker invalidates the phase-08 mutation results.

## Debt

- **DEBT-044 discharged** — `git add -f` on the five parity-suite files; `git ls-files` = 5. The
  "Secondary" half (un-sort the differ's keys) was already done at the phase-08 reflect:
  `diff_bridge_contract.py` uses `sort_keys=False` in both `canon` and `normalize`.
- **DEBT-043 dropped by the owner** (2026-11-26): a Cloudflare quick tunnel's temporary domain is
  acceptable, so the named tunnel is not owed. Note in the row records that AT-STARTUP hardening is
  *not* dropped — it is 09-02/09-03's `install_stack` work, carried by the phase plans.
- **DEBT-045 filed** (new): the `check-mt5c.sh` live-clock assertion.

## Security review (the phase-09 override row, run at UNIFY)

The `/security-review` skill's body is written for a C# blockchain codebase, so its *principles* were
applied to this diff rather than its checks: hash-input completeness, concurrency/atomicity, closed
deserialization, boundaries, and no key material in logs. Two real findings, both fixed here.

**S1 — pairing codes were never swept (Medium-High, fixed).** The plan told me the existing sweep's
cascade covers expiry: "the existing sweep's cascade already removes expired rows; reuse it, do not
add a second sweeper". That is false for a reason worth stating precisely. S14 is
`DELETE FROM sessions WHERE expires_at < $1` — it removes expired **sessions**. A pairing code is
`ON DELETE CASCADE` from its minting session, and that session lives **30 days**; the code lives
**10 minutes**. So every minted-but-unused code sat in `pairing_codes` for a month: a 10-minute
credential retained ~4,320x its lifetime, growing the table with the mint rate (the per-IP bucket
alone permits 12,000 rows per session per TTL window). Not exploitable — `resolve_pairing_code`
refuses on `now >= expires_at` before anything else — but dead-credential retention at that multiple
is exactly what an expiry sweep exists to prevent.

Fixed: `P4 = DELETE FROM pairing_codes WHERE expires_at < $1`, called from `Auth::sweep_expired`
alongside S14 (one sweep, two statements, one interval — no second sweeper task). Two tests pin it:
`the_sweep_removes_expired_pairing_codes` (advance 11 minutes: session alive, code gone, and the
assertion names why no cascade could have done it) and `an_expired_code_refuses_even_before_any_sweep`
(the sweep is housekeeping, not the security boundary). Negative control run: replacing the P4 call
with `let codes: u64 = 0` turned exactly one test red.

**S2 — the new FK had no index (Low, fixed).** 0001 and 0003 both index their FK columns
(`identities_user_id`, `sessions_user_id`, `mt5_bridges_user_id`); 0004 as planned did not. Without
it, S9 (sign out) and S14 must scan `pairing_codes` per deleted session to enforce the cascade.
Fixed: `pairing_codes_session_id_hash` and `pairing_codes_expires_at`, both in 0004 (the migration
had not been committed, so it was still mutable — see the immutability rule below).

**Verified clean, with the reasoning rather than a tick:**

- **No SQL injection surface.** Every new statement is a `const &str` with bound placeholders; no
  `format!` reaches SQL anywhere in `crates/hub-core/src/`.
- **Secret entropy and brute-force budget.** The code space is 10^10 (~33.2 bits). At D-007's 20/s
  per-IP bucket over the 10-minute TTL that is 12,000 guesses per IP per window = 1.2e-6 per IP;
  even 1,000 IPs land at 1.2e-3. Uniform by rejection sampling, so no residue is likelier.
- **Secrets never logged.** `PairingCode`, `RevokeSecret`, `PairingCodeHash`, `RevokeHash` all have
  hand-written `Debug` printing only the type name; no `tracing::` call in `routes/pairing.rs` or the
  new `Auth` methods mentions a code or secret; both the minted code and the revoke secret join the
  ring's redaction set via `add_secret`; the revoke path calls `add_secret` on the *wrong* secret too,
  so a mistyped one is redacted as well.
- **No new unauthenticated hole.** The two Origin-exempt paths carry their own credential and mint no
  session; `/api/me/mt5/pairing-code` stays behind both the gate and the cookie.
- **Constant-time comparison.** The hash comparisons happen in SQL (`WHERE revoke_hash = $2`), which
  is the same pattern S12/S13 already use for desktop codes — a hash comparison, not a secret
  comparison, so the timing channel reveals nothing usable. `KeyHash::ct_eq` remains for the one place
  that compares a presented key against a stored hash in Rust.
- **Migration immutability.** 0004 had never been committed or applied anywhere but this box's test
  databases, so editing it in place was legitimate; had it been committed, S2 would have needed a
  0005.

## Skill audit

| Skill | Invoked | Notes |
|---|---|---|
| context-budget, caveman, ponytail | yes | AGENTS.md rule, loaded at session start |
| pstack (`pstack_stage: plan`) | n/a | APPLY ran in-session; no poteto-mode gating per config |
| /paul-security | no | see below — the security review is the /security-review row at UNIFY |
| /modern-web-guidance (dialog, validate-origins) | partial | native `<dialog>` per D-020 (browser owns trap + Esc); same-origin fetch only. Guide bodies not re-retrieved this session — the patterns were already in `settings.js`/D-020 |
| /a11y-audit, /huashu-design | yes | gate_all 8/8 × 5 pages served, axe 0 |
| /security-review (phase-09 override) | yes | run at UNIFY over the whole plan diff; two findings (S1 sweep, S2 FK index) fixed here, four properties verified clean — see the Security review section |

## Modern Web Guidance

- **`dialog`**: the pairing dialog is a native `<dialog>` opened with `showModal()`, so the browser
  owns the focus trap, the inertness of the page behind it, and Esc. Rule applied rather than
  re-derived: the platform provides the behaviour a hand-rolled modal would have to reimplement.
  The `close` event is the single teardown point — it clears the code out of the DOM, which matters
  because the code is a live secret.
- **`validate-origins`**: the page fetches only same-origin paths (`/api/me/mt5/pairing-code`); no
  absolute URL enters the client. The clipboard write fires only from the button's own click, never
  at load.

## Reflect and RSI

`deferred to phase close (reflect: pstack)` — 09-01 is not the phase's last plan (09-03 is), so the
phase-scope panel runs at 09-03's UNIFY. This plan's own findings were recorded as they landed
instead of waiting: L-055 (validated) and L-056 (candidate) are in `.paul/LEARNINGS.md`, and the two
security findings are in the section above.

`rules: +0, decisions: +0, learnings: +2 added, 0 bumped, 0 promoted, 0 removed/narrowed`
`debt: +1 added (DEBT-045), 1 discharged (DEBT-044), 1 dropped (DEBT-043)`

**UNIFIED ≠ landed: 40 files uncommitted** until the owner commits (auto_commit is off).

## Open for 09-02

- `pair` returns `revoke_code` exactly once; the app must persist `{id, revoke_code}` and never log it.
- The two code-auth paths need no Origin and no cookie — a client sending either still works, but
  neither is required, and no session is minted by either.
- A pairing code is spent by the *attempt*, not by the success: a guard refusal (422) burns it too.
  The app's retry must mint a fresh code, or the UI must say so plainly.
