# 05-01 Summary — Migration 0002 user_state, statements S15..S21, Auth state methods

**Status:** COMPLETE (2026-11-25, uncommitted at pause; wave 1)
**Plan:** `.paul/phases/05-sync/05-01-PLAN.md`

## Task statuses

| Task | Status | Evidence |
|------|--------|----------|
| 1 — Migration 0002 + `MIGRATIONS` entry | done | `0002_user_state.up.sql`/`.down.sql`; both engines apply/reverse green |
| 2 — Statements S15..S21, shape count 14 → 21 | done | `every_store_statement_is_single_table` green at 21 |
| 3 — `Auth::state_all/state_put/state_delete` + `tests/state.rs` | done | 8 tests green on both engines; mutations proven |

## Statements as shipped

`S15` list, `S16` read one, `S17` insert `ON CONFLICT DO NOTHING`, `S18` compare-and-swap update
(`AND version = $6`), `S19` delete one, `S20` delete one at a bound version (cap rollback),
`S21` expire backups. **S21 shipped as the two-bound text range** (`key >= $2 AND key < $3`,
`$2 = "backup:"`, `$3 = "backup;"`) over the `LIKE` form: `sql_shape_violations` accepts both, and the
range needs no pattern-escaping reasoning. `version` is INT4 — `i32` on the Rust side (plan-fact
correction 1; the plan's `StateRow` sketch said `i64`).

## Mutation run (m1 and m1b)

- **m1** — weaken `S18`'s version predicate (the plan's named mutation): **changed nothing**. The Rust
  guard (`expected == row.version`) rejects the stale write before `S18` executes, so AC-3's test still
  passes with the SQL predicate broken. Recorded as an observation, not a failure of the suite: the
  layering means the SQL predicate is defence in depth, the Rust check is the load-bearing guard.
- **m1b** — weaken the Rust version check to `>` (a stale-or-equal write succeeds):
  **failed 2 tests**, restore landed (grep), file `touch`ed (L-013), suite green again. The conflict
  path is proven to be able to fail.

## Engines and counts

`tests/state.rs`: 8 tests, green on **both** engines (`TestDb::start` reported pgsqlite and Postgres 17).
Migration shape walk (`every_statement_and_migration_is_single_table`) green; `--test db_engines`
apply/reverse green on both engines (9 passed, 2 ignored — the second engine's rows skip where the
box has no second engine process).

**Verify-set finding (2026-11-25 session):** `tests/state.rs` was missing its `[[test]]` entry in
`crates/hub-core/Cargo.toml`, so bare `cargo test`/`clippy --all-targets` built it without
`test-idp` and failed with 6 unresolved-symbol errors (it uses the `hosted` harness, which imports
`auth::test_idp`). Fixed by adding `[[test]] name = "state" required-features = ["test-idp"]`,
matching db_engines/auth/admin/gate. The suite then ran only under `--features test-idp`
(`cargo test -p hub-core --features test-idp --test state` → 8 passed), and clippy is clean both bare
and with `--all-features`.

## Plan-fact corrections (2)

1. `version` is INT4 (i32), not i64 — the plan's `StateRow` sketch was wrong.
2. `db_engines.rs` pinned a one-migration world; updated for two migrations.

## Decisions recorded

- **D-024** (256 KiB value cap, `VALUE_MAX = 262_144`) — full entry already in `.paul/DECISIONS.md`
  (transcribed this session by the crashed 04-03 session's successor; verified present).

## Deviations

None beyond the two fact corrections above. No new dependency, no route, no client file touched.
`pstack: disabled at APPLY (pstack_stage: plan); plan skills (architect) ran at plan stage` — per config,
recorded for the skill audit.

## Next-phase readiness

Storage layer ready for 05-02 (HTTP surface): `state_all/put/delete` + `PutOutcome` are the contract
05-02's routes wrap; 05-03 depends on 05-02's table, not this plan directly.
