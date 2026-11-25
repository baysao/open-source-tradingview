# 05-02 Summary — The `/api/me/state` HTTP surface, the refusal contract, and the daily backup prune

**Status:** COMPLETE (2026-11-25, uncommitted; wave 2)
**Plan:** `.paul/phases/05-sync/05-02-PLAN.md`

## Task statuses

| Task | Status | Evidence |
|------|--------|----------|
| 1 — Four handlers + route specs | done | `routes/state.rs`; `pub mod state;`; four `hosted()` specs at `routes/table.rs:65-80` |
| 2 — Gate enumeration fix for `{key}` | done | `sample_path()` at `tests/gate.rs:24-42`; `!= 404` clause at `tests/gate.rs:141-149` |
| 3 — HTTP contract tests + daily prune | done | `tests/state_routes.rs` 9 tests; `spawn_background` prune at `src/lib.rs:376-394` |

## The contract as shipped

Identical to the plan's table. 05-03 may code against it unchanged.

| Call | Request | Success | Failure |
|---|---|---|---|
| `GET /api/me/state` | — | 200 `{"keys":[{"key","version","updated_at"}]}` | 401 `sign-in required` |
| `GET /api/me/state/{key}` | — | 200 `{"key","value","version","updated_at"}` | 401; 404 `not found` |
| `PUT /api/me/state/{key}` | body `{"value":<string>}`, `If-Match: <n>` | 200 `{"key","version"}` | 409 `{"error":"version conflict","version","value"}`; 413 `value too large`; 409 `too many keys`; 400 `bad key`; 428 `if-match required`; 400 `bad if-match`; 401 `sign-in required` |
| `DELETE /api/me/state/{key}` | — | 204 | 401 `sign-in required` |

Exact error strings shipped, in the order each route evaluates them:

| Route | Order |
|---|---|
| all four | 401 `sign-in required` (viewer resolved first) |
| `get_one` | then 404 `not found` for an invalid key or a missing row |
| `put` | then 400 `bad key`, 428 `if-match required`, 400 `bad if-match`, then the storage outcome |
| `delete` | then 204 for an invalid key (a key not writable is a key not present) |

`version` is `i32` (INT4, 05-01's correction 1). `expected == 0` = insert. `If-Match` is trimmed,
`W/`-stripped and `"`-stripped before `parse::<i32>()` (RFC 9110 §13.1.1). `updated_at` is RFC 3339.

## AC results

| AC | Test |
|---|---|
| AC-1 | `gate::every_hosted_route_is_in_a_gate_area`, `gate::origin_rules_over_every_hosted_non_get` |
| AC-2 | `state_routes::list_omits_values` (asserts the serialized field set, `chartHub` at 3, `chartHubTheme` at 1) |
| AC-3 | `state_routes::stale_put_is_a_409_carrying_the_current_value_and_writes_nothing` (view 7, If-Match 5) |
| AC-4 | `state_routes::current_put_stores_and_advances` (7 → 8) |
| AC-5 | `state_routes::refusals_are_specific`, `state_routes::too_many_keys_is_a_409` |
| AC-6 | `state_routes::signed_out_is_401_on_all_four` (driven from `route_specs`; row count 0 after) |
| AC-7 | `state_routes::delete_is_idempotent` |
| AC-8 | `state_routes::the_prune_deletes_only_old_backups` |
| AC-9 | `state_routes::cross_user_read_is_impossible` (+ no `{user` path in the table) |

## Mutation run — both plan-named mutations cannot bite as written

The plan named two mutations. **Neither the plan's form failed the suite**, for two different
reasons, and each was replaced by a mutation of the same claim that does:

1. **Task 2's mutation (`state::put` returns 403 unconditionally): no effect.**
   `axum::Json<PutBody>` is an extractor: it runs *before* the handler body and rejects the gate
   test's `{}` body with 422, so the mutant line never executes. The claim under test is "the gate
   lets the request reach the handler", so the mutation was moved to the mount:
   `routes/table.rs` mounted `mutant_put` (403 for every PUT).
   **Result: `origin_rules_over_every_hosted_non_get` FAILED** at
   `tests/gate.rs:140` — `PUT /api/me/state/{key} with PUBLIC_ORIGIN: left: 403, right: 403`.
   Restored, grepped clean (`ygrep mutant` → 0 results), both files `touch`ed (L-013), green again.
2. **Task 3's mutation (cutoff 0 days): no effect.**
   The AC-8 test calls `Auth::expire_backups(cutoff)` with its own explicit 30-day cutoff, so the
   literal in `spawn_background`'s loop is not on the tested path — which is the point of exposing
   the body. The mutation was moved to the statement the test drives: `S21`'s `AND key >= $2` was
   weakened to `OR key >= $2` (the backup-prefix filter removed).
   **Result: `the_prune_deletes_only_old_backups` FAILED** — `left: 3, right: 1` (every row
   matched the age predicate). Restored, grepped clean, `touch`ed, green again.

Both mutations were **re-run after** the AC-5/AC-8 checker edits (R-10: a checker edited is a
checker re-proven) and failed both times before the final restore.

The gate mutation matters for a second reason: it confirms `sample_path` substitution is live. In
the mutation run `gate.rs:140` fired on `{key}`, which can only happen if the URL reached a mounted
route.

## Engines and counts

`tests/state_routes.rs` — 9 tests, green on **both** engines (`VTV_TEST_PG_URL` set to a PostgreSQL
18.7 instance; without it the 9 Postgres passes SKIP). `tests/state` 8, `tests/gate` 7,
`tests/admin` 1, all green both engines. `cargo test --workspace` (with `hub-core/test-idp`):
**192 passed** (the 03 exit floor was 126 in check-0402; the crate-test count is now 192).
`cargo fmt --all --check` clean bare; `clippy --workspace --all-targets -D warnings` clean.

`L-026` check: `-- --list` reports 9 tests before the run, so a renamed module cannot pass at zero.

## Verify-set findings (plan-fact corrections, 3)

1. **Task 2's mutation as written cannot fail** (extractor ordering). Recorded above; the plan's
   `<verify>` text should name the mount, not the handler, when the route has a body extractor.
2. **Task 3's mutation as written cannot fail** (the explicit-cutoff seam moves the literal off the
   tested path). Both are a property of the plan asking for a mutation of the *deployment* rather
   than of the *decision*.
3. **AC-5's `/` key case is reachable only percent-encoded.** A literal `/` is a second path
   segment, so `PUT /api/me/state/a/b` is a 404 from the router. `%2F` matches `{key}` as one
   segment and decodes to a slash, which is the input AC-5 names; the test now asserts it.
4. **Postgres was not running on this box.** The earlier `state`/`gate` runs were pgsqlite-only with
   the Postgres half SKIPPED — 05-01's SUMMARY says "both engines" and 05-02's plan requires it, so
   a throwaway PG 18.7 cluster was initialised in `%TEMP%/vtv-pg18` on port 55433 for this session.
   Nothing in the repo points at it; `VTV_TEST_PG_URL` is the only switch and CI already sets it.

## Security review (loaded before Task 1, applied to the handler diff)

Two findings, both fixed in this plan:

1. **Ordering defect — `get_one` and `delete` checked the key before the viewer.** AC-6 says a
   signed-out caller gets 401 on every route; answering 404 (get_one) or 204 (delete) first makes
   the signed-out answer depend on the input. Both were reordered to resolve the viewer first, as
   `put` already did, and `signed_out_is_401_on_all_four` now drives a *valid-looking* key so the
   order is the thing under test.
2. **AC-6's "none of them writes a row" was unasserted.** Added: the `user_state` row count is 0
   after all four signed-out calls.

Reviewed and accepted as-is: the value/key caps are enforced in `Auth::state_put`
(`VALUE_MAX` before any statement; the key cap corrective via `S20` at the written version, so a
concurrent writer's row is untouchable); every statement is parameterised and single-table; no
user id is ever taken from the request (AC-9 asserts no `{user` path exists); the 409 body carries
only the caller's own row; `PUT` is inside the gate's Origin + `application/json` + 300,000-byte
rules, none of which this plan re-implements.

One observation left as a debt row (DEBT-033): the `too many keys` refusal is corrective, so a
burst of concurrent inserts from one user can transiently hold 257+ rows before the undo lands.
Bounded by the per-IP bucket, not exploitable as a quota bypass in practice, but it is not the
preventive check the spec's wording implies.

## Deviations

- No dependency added, no gate branch, no new bucket, no CSP entry, no client file touched.
- `routes/gate.rs`, `routes/me.rs`, `net.rs`, `static_files.rs`, `auth/store.rs` untouched per the
  boundaries. `S21` was temporarily mutated for the mutation run and restored byte-identically
  (`grep -c "OR key"` → 0).
- `Auth::expire_backups` already existed (05-01 shipped it); this plan only added its caller.

## Deferred (view of the store)

- **DEBT-033** — the key cap is corrective, not preventive (owner: agent, trigger `phase:05`).
- **DEBT-034** — `/api/me/state` has no documented contract in `docs/`; 05-03 was to carry it
  (owner: agent, trigger `phase:06`), so the plan's contract table is the only reference until then.

## Next-phase readiness

05-03 (client) can code against the table above with no reservation: every status, every error
string and every field name it will read is asserted by a test that was proven able to fail.
