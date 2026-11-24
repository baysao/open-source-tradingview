---
phase: 03-auth
plan: 01
status: UNIFIED
completed: 2026-11-24
applied: 2026-11-24
type: Summary
about: "vtradingview"
---

# 03-01 SUMMARY: hosted config, the database layer, migration 0001

## Result

`Db::open(url)` connects to pgsqlite or Postgres and applies the embedded migrations with our own
runner: no `IF NOT EXISTS`, no advisory lock, one transaction per file. The R3 schema is proven on
the owner's pgsqlite 0.0.22: up, up again, down, up; constraints; cascades; round trips. Every
connection is checked before use, because pgsqlite corrupts a connection after any statement error
(D-018). `Config::from_env` parses hosted mode, names every missing variable at once and redacts
secrets. `AppState::build` still refuses hosted mode until 03-02. Postgres was not run locally (no
server on this PC); the new CI `db` job runs it with skips turned into failures (DEBT-005).

## Tasks

| Task | Status | Qualify |
|---|---|---|
| 1. Engine harness, db.rs, migration 0001, db_engines suite | DONE_WITH_CONCERNS | PASS after two fixes: the pgsqlite desync (deviation 1) and text-literal times (deviation 2) |
| 2. Hosted config, redaction, AuthMode callers | DONE | PASS: 9 config tests with and without `test-idp`; security review below |
| 3. db process line, .env.example, CI engines, mutations | DONE_WITH_CONCERNS | PASS, except that the db listens on 0.0.0.0 (DEBT-011); two mutations were first blind (deviation 7) |

Schedule: sequential (Tasks 1 and 2 share `lib.rs` and one cargo target).

## Acceptance criteria

| AC | Result | Evidence |
|---|---|---|
| AC-1 hosted config | pass | `hosted_names_every_missing_variable_at_once` (four names, spec order, exact Display text); `PUBLIC_ORIGIN` http refused; 31-byte and non-base64 keys `Invalid` with `<redacted>`; `ADMIN_EMAILS " A@x.io, b@y.io ,,"` gives `["a@x.io","b@y.io"]`; `debug_shows_no_secret`; `idp_urls_follow_the_feature` (ignored without `test-idp`, read with it); `build_refuses_hosted_mode_until_auth_exists`; `bind_addr` 127.0.0.1 in both modes |
| AC-2 runner on both engines | pass on pgsqlite; Postgres not run locally | `up_up_down_up` ([1], [], empty table after down, [1]); `refuses_edited_unknown_and_partial` (ChecksumChanged 1, UnknownApplied 1, Migration 2 with no row and no `t_partial`); `a_malformed_migrations_table_is_an_error_not_absent`; `checksum_ignores_crlf`; pure `plan_migrations` units |
| AC-3 schema rules | pass on pgsqlite | `representative_rows_round_trip` (NULL in every nullable column, microsecond timestamptz, bytea, typed binds); `absent_table_is_detected_but_other_errors_are_not` (`is_unique_violation` on pgsqlite's 42000 message); `constraints_and_cascades` (FK refused; user delete empties identities, sessions, codes) |
| AC-4 shape, files, require | pass | `every_statement_and_migration_is_single_table` over M1, M2, M4, M5 and both 0001 files; `shape_rule_flags_each_banned_form`; `test_files_stay_in_the_temp_dir`; `a_required_engine_that_is_absent_fails` (required: child fails "pgsqlite is required"; optional: SKIPPED and passes) |
| AC-5 checks can fail | pass (18 of 18) | Mutation log below; 16 failed on the first run, 2 were blind and fixed (deviation 7), then failed |
| AC-6 local mode, dev tooling | partial | Gates: fmt 0, clippy 0 (default and `--features test-idp`), `cargo test --workspace` 0 (96 passed, 0 failed), `cargo test -p hub-core --features test-idp` 0. `_svc db` starts pgsqlite from `~/.local/bin/pgsqlite` on 55432 over `data/vtv.db`, but it listens on **0.0.0.0**: pgsqlite 0.0.22 has no listen-address option. `dev.sh` warns and will pass `--host 127.0.0.1` once the binary supports it (DEBT-011, blocks Phase 03 exit) |

### Mutation log (AC-5, L-003, L-009, L-013)

Script: `scratchpad/p03db/mutate_0301.py`. Each mutation is applied and grep-checked, the named test
is run (it must FAIL), then the file is restored and touched.

| # | Mutation | Test that failed |
|---|---|---|
| 1 | `Missing` reports only the first | `config::tests::hosted_names_every_missing_variable_at_once` |
| 2 | accept an `http://` origin | `config::tests::hosted_refuses_http_origin_and_short_key` |
| 3 | accept a 31-byte key | same |
| 4 | read `GOOGLE_JWKS_URL` without the cfg gate | `config::tests::idp_urls_follow_the_feature` |
| 5 | `Debug` prints the `Secret` | `config::tests::debug_shows_no_secret` |
| 6 | skip the M2 check (always create) | `up_up_down_up` |
| 7 | treat any M2 error as absent | `a_malformed_migrations_table_is_an_error_not_absent` (after deviation 7) |
| 8 | drop one `DROP TABLE` from the down file | `up_up_down_up` |
| 9 | skip the checksum compare | `refuses_edited_unknown_and_partial` |
| 10 | run M4 outside the file's transaction | same |
| 11 | remove the CRLF normalization | `db::tests::checksum_ignores_crlf` |
| 12 | write nanosecond times | `representative_rows_round_trip` |
| 13 | match `23505` only | `absent_table_is_detected_but_other_errors_are_not` |
| 14 | remove one `ON DELETE CASCADE` | `constraints_and_cascades` |
| 15 | `IF NOT EXISTS` in M1 | `db::tests::every_statement_and_migration_is_single_table` |
| 16 | test file under `CARGO_MANIFEST_DIR` | `test_files_stay_in_the_temp_dir` |
| 17 | SKIPPED regardless of `VTV_TEST_REQUIRE` | `a_required_engine_that_is_absent_fails` |
| 18 | pool guard accepts a connection whose probe failed | `a_failed_statement_does_not_poison_later_writes` (after deviation 7) |

### Measured (not gated)

- pgsqlite start-up per test, from spawn to the first connection: **0.52 s** mean over 10 runs.
  `Db::open` with 0001 on a fresh file: **0.18 s**. The `db_engines` suite takes 2.3 s. Per-test
  isolation (option a) stays.
- Two write transactions on one file: the second insert **waits about 5.5 s, then fails** with
  `42000 "Execute failed: SQLite error: database is locked"`; both commits then return Ok, and one
  row lands. 03-02 must treat this error as retryable in the sign-in effect and keep transactions
  short.

## Deviations (each verified)

1. **pgsqlite desynchronizes a connection after any statement error (new D-018).** After an error
   (for example a foreign-key failure), every later reply on that connection arrives one message
   late. A re-executed cached INSERT reported `Ok(0)` and wrote nothing; `SELECT 1` failed with
   "expected ParseComplete, got ReadyForQuery". This happens on the owner's 0.0.22 and on the fork,
   and turning the statement cache off does not help. Fix: `db::pool_options()`, which every pool
   uses, probes each connection with `SELECT 1` before handing it out and closes a poisoned one.
   Closing rolls back its open transaction and leaves other connections writable (measured).
   Regression test: `a_failed_statement_does_not_poison_later_writes` (pool of 1, three error
   rounds, an error inside a transaction).
2. **A `timestamptz` written as a text literal is stored as text,** and a binary decode later
   fails ("Value Buffer exceeds 8 bytes"). The fixture rows are now written with typed binds,
   and nothing in the server writes times as literals.
3. **`DbError::Schema`** was added (not in the sketch) for an M1 or M2 failure, so a damaged
   `schema_migrations` is named, not reported as a migration.
4. **`PUBLIC_ORIGIN` is part of the hosted `Missing` list,** because the spec requires an https
   origin with auth on. AC-1's four-name case still holds (its fixture sets the origin).
5. **The `base64 0.22` crate was added here** for `VTV_SECRET_KEY`; 03-02 reuses it.
   `db::sql_shape_violations`, `db::split_statements` and `db::now_micros` are public so the
   tests (and the 03-02 store test) can use them.
6. **CI builds the owner's pgsqlite fork pinned to commit `5e34b2d`** (`cargo install --rev`,
   cached by that rev). The owner's local binary is 0.0.22, the stricter of the two, and the
   suite is green on it. Pinning was a security-review finding: an install from a moving branch is
   not reproducible.
7. **Two mutations were blind on the first run, and both were fixed:** (7) the test asserted
   only that `checksum` appeared in the error text, and the mutated path's error ("already exists")
   quotes the whole SQL, which contains that word too; the test now asserts the error kind
   (missing column, never "already exists"). (18) `v == 1` to `true` is an equivalent mutation:
   on a poisoned connection the probe errors, so the value is never compared. It was replaced by
   `unwrap_or(true)`, which accepts a connection whose probe failed, and that mutation fails the test.
8. **One of my own tests wrote a file.** `build_refuses_hosted_mode_until_auth_exists` first also
   built a local `AppState`, which opened `crates/hub-core/hub/logs/hub.log` (L-008). The stray
   file was found by `git status`, the local half was removed, and the file was deleted.
9. **The `measure_` test is `#[ignore]`.** Run it with `-- --ignored --nocapture measure_`.

## Security review (Phase 03 override, Task 2; also covers db.rs and Task 3)

The `/security-review` skill installed in this environment targets another codebase: its
checklist covers block hashing, `Block<T>` and Newtonsoft.Json in C#, so none of it applies here.
I loaded it and recorded it as not applicable. I reviewed the diff against this plan's concerns
instead.

| Finding | Severity | Disposition |
|---|---|---|
| pgsqlite listens on `0.0.0.0` with no password; no listen-address option | High (dev host on a shared network) | DEBT-011, blocking Phase 03 exit; `dev.sh` warns and switches to `--host 127.0.0.1` once the binary supports it. The fix belongs in `~/project/pgsqlite` (owner's request) |
| Connection errors could print the `DATABASE_URL` password | none found | Probe: an unreachable host, an unknown host and a malformed URL all print the sqlx cause with no URL |
| `Debug` of `Config` and `AuthSettings` | none | `Secret` and `SecretKey` print `<redacted>`; test plus mutation 5 |
| `VTV_SECRET_KEY` in errors | none | `Invalid` stores `<redacted>`; the test checks the Display text too |
| IdP URL override in release | none | `GOOGLE_*` are read only with `test-idp`; test plus mutation 4 |
| CI installs pgsqlite from a moving branch | Medium | Fixed: pinned `--rev 5e34b2d` |
| `Secret` derives `PartialEq` (not constant time) | Low | Not used to compare secrets. 03-02 compares hashes and keys with `subtle` |

## Skill audit

| Skill | Result |
|---|---|
| /architect | yes (plan): `03-ARCHITECTURE.md` |
| /security-review | loaded; checklist not applicable (C# blockchain skill), manual review above |
| /paul-security (optional) | guidance folded into the review table (new crates: sqlx, uuid, chrono, sha2, base64, tempfile) |
| pstack | `pstack_stage: plan`; APPLY ran in-session |

No front-end file was touched, so no Modern Web Guidance or uxui rows apply.

## Deferred (view of the store)

- DEBT-011 (new, blocking phase-03-exit): the dev db must listen on 127.0.0.1 only.
- DEBT-005 (waiting): the first CI run, which now includes the `db` job on Postgres 17 and pgsqlite.

## Learning candidates for UNIFY

- pgsqlite poisons a connection after any statement error: check connections before use, and
  write a regression test with a pool of 1.
- An error-text assertion can be satisfied by the wrong path's error when that error quotes the
  SQL: assert the error kind. A mutation whose path fails the same way as the original is
  equivalent: mutate the decision that actually discriminates (extends L-003).
- `git status` after a test run is the cheapest L-008 check. It caught a test of mine writing
  `hub.log`.

## Open risk

**APPLIED is not LANDED.** Every file below is uncommitted until the owner commits. The Postgres
half of every engine test has not run anywhere yet (DEBT-005).

## Files

Created: `crates/hub-core/src/db.rs`, `crates/hub-core/migrations/0001_users.{up,down}.sql`,
`crates/hub-core/tests/common/db.rs`, `crates/hub-core/tests/db_engines.rs`. Modified:
`crates/hub-core/src/{config,lib}.rs`, `crates/hub-core/Cargo.toml`, `Cargo.toml`, `Cargo.lock`,
`.env.example`, `Procfile`, `scripts/dev.sh`, `.github/workflows/ci.yml`.

## Next-plan readiness (03-02)

Ready, with four inputs: (1) any statement error ends its transaction; propagate it, and the pool
guard replaces the connection (D-018); (2) `42000 database is locked` after about 5.5 s is the
retryable busy error; (3) `Db::pool()` is crate-private for `auth/store.rs`, whose `STATEMENTS`
join the shape test through `db::sql_shape_violations`; (4) `AuthNotAvailable` is deleted when
`AppState::start` exists.

## Reflect and RSI

- reflect: deferred to phase close (reflect: pstack); 03-01 is plan 1 of 3.
- instruction edits proposed: none.
- rsi: deferred to phase close (reflect: pstack); the 3 learning candidates above stay in
  SUMMARY for the phase close.
