---
phase: 04-deploy
plan: 01
completed: 2026-11-24T21:40:00+07:00
duration: ~3h (two sessions: planning, then APPLY resumed)
description: "Deploy surface: the ops/ file set for the VPS (systemd units, tunnel config, nightly R2 backup, restore drill, deploy script, structural checker), the tunnel line in Procfile and dev.sh, the load test, and the .txt static extension"
type: Summary
about: "vtradingview"
---

# 04-01 SUMMARY: deploy surface (ops/, tunnel line, load test, .txt extension)

**Status:** UNIFIED. All five tasks applied; reconciled at UNIFY 2026-11-24. No checkpoints in this plan; the two owner rows it
creates (DEBT-018, DEBT-019) are the launch acts, not gates on this APPLY.

---

## What shipped

| Area | Files | State |
|---|---|---|
| Ops surface | `ops/README.md`, `ops/cloudflared.yml`, `ops/vtv.service`, `ops/vtv-backup.service`, `ops/vtv-backup.timer`, `ops/backup.sh`, `ops/restore_drill.sh`, `ops/deploy.sh`, `ops/check.py` | new |
| Tunnel line | `Procfile` (+`tunnel:`), `scripts/dev.sh` (`svc_known`, `ALL_SERVICES`, `_svc tunnel`, header comment) | edited |
| Load test | `scripts/load_ws.mjs` | new |
| Static extension | `crates/hub-core/src/static_files.rs` (`STATIC_EXT` + 5 test cases) | edited |
| AGPL placeholder | `hub/SOURCE.txt` | new |
| Debt | `.paul/DEBT.toml` +DEBT-018, +DEBT-019 | edited |

### ops/ in detail

- `cloudflared.yml`: tunnel `vtradingview`, exactly one ingress rule (`vtradingview.com` to
  `http://127.0.0.1:8770`) plus a `http_status:404` catch-all. YAML parses; the ingress shape is
  asserted by `ops/check.py`.
- `vtv.service`: user `vtv`, `EnvironmentFile=/etc/vtv/env`, `NoNewPrivileges`,
  `ProtectSystem=strict`, `ProtectHome=read-only`, `PrivateTmp`, `ReadWritePaths` limited to the
  hub log and orderflow cache, `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`, empty
  `CapabilityBoundingSet`, `Restart=always`, `MemoryDenyWriteExecute`, `LockPersonality`.
- `vtv-backup.service` + `vtv-backup.timer`: nightly 04:00 UTC, `Persistent=true`, network
  runtime only.
- `backup.sh`: sources `/etc/vtv/env`, `pg_dump --format=custom`, zero-byte guard (refuses to
  upload an empty dump), `rclone rcat` to R2, `rclone delete --min-age 14d`, `--check` structural
  self-test.
- `restore_drill.sh`: newest R2 object, `pg_restore` into a date-stamped scratch database (never
  the production one), asserts `schema_migrations` versions against
  `crates/hub-core/migrations/*.up.sql`, prints per-table row counts, boots the release binary
  against the scratch on a scratch port and curls `/api/health` + `/auth/me`, kills only its own
  PID, prints PASS/FAIL and exits nonzero on any failure, `--check` structural self-test.
- `deploy.sh`: fetch + detach checkout, release build, previous binary to `hub-server.old`,
  writes `hub/SOURCE.txt` with the exact commit/build date/repo URL (the AGPL offer), restarts
  `vtv`, smokes the public `/api/health`, prints the rollback command.
- `README.md`: the runbook (VPS prep, Postgres, env-file contract with rotation consequences,
  tunnel creation, unit install, backup/drill cadence, AGPL note, owner-row list, verification
  checklist).
- `check.py`: the structural gate the plan's AC-1/AC-2 needed. Parses the unit, the tunnel
  config, runs both scripts' `--check`, and scans for literal secrets. It is also what makes
  mutation m2 possible without a cloudflared or systemd install.

## Acceptance criteria

| AC | Result | Evidence |
|---|---|---|
| AC-1 ops set complete and consistent | pass | `python ops/check.py` OK: 9 files present, 10 unit properties exact, tunnel ingress exact, both script self-tests green, no literal secrets. `systemd-analyze verify` SKIPPED: no systemd on this Windows machine; the equivalent parsed check is in `check.py` |
| AC-2 drill restores, asserts, cleans up | pass (structural), run-time deferred | `--check` verifies every assertion is present and the restore targets `$SCRATCH_URL`; the actual restore needs a live Postgres + R2 (DEBT-023) |
| AC-3 tunnel line through the same paths | pass | Without cloudflared: exact message + idle, no crash loop (exit 124 from the test timeout only). With cloudflared 2026.11.2 installed mid-session by the owner: `exec` ran and stopped only on the missing origin certificate, i.e. the branch executes and the config path is right |
| AC-4 load test states its mix and passes | pass at 60 connections; the 1,000 target deferred | 60/60 held, 0 dropped, 0 connect errors, 46 upstreams for 50 subscribed symbols, deriv refusing this runner (skip-adjusted, exit 0). 1,000 needs several source IPs or a raised cap: DEBT-022 |
| AC-5 txt serves; shape not widened | pass | `/SOURCE.txt` 200 live; `/../SOURCE.txt` and `/a/b/SOURCE.txt` still refused; `/x.txt` refusal case replaced by `/x.py` (txt is now intentionally allowed at top level) |
| AC-6 checks can fail | pass | m1..m5 all red-then-green, with the checker defects m4 exposed fixed (see Deviations) |

## Mutations (AC-6)

| Id | Mutation | Check that bit | Result |
|---|---|---|---|
| m1 | `".txt"` removed from `STATIC_EXT` | `cargo test -p hub-core static_files` | FAIL then PASS |
| m2 | 404 catch-all removed from `cloudflared.yml` | `python ops/check.py` | FAIL then PASS |
| m3 | zero-byte guard disabled in `backup.sh` | `bash ops/backup.sh --check` | FAIL then PASS |
| m4 | row-count assertion stubbed in `restore_drill.sh` | `bash ops/restore_drill.sh --check` | FAIL then PASS |
| m5 | load test's pass line inverted | a clean 8-connection run | FAIL then PASS |

Driver: `.paul/runs/mutate-0401.sh`. L-009 honoured (each edit grepped as landed before the run);
L-013 honoured (touched after restore); R-10 honoured (the two checker edits re-ran the full set).

## Gates (bare exit codes, R-8)

- `cargo fmt --all --check` exit 0
- `cargo clippy --workspace --all-targets -- -D warnings` exit 0
- `cargo clippy -p hub-core --all-targets --features test-idp -- -D warnings` exit 0
- `cargo test --workspace` exit 0 (118 passed across suites)
- `cargo test -p hub-core --features test-idp` exit 0 (144 passed, 3 ignored)
- `bash -n` on all three scripts exit 0; `node --check scripts/load_ws.mjs` exit 0; the YAML parses

## Deviations

1. **`ops/check.py` was added; the plan did not name it.** The plan's m2 mutation ("run the
   README config greps") had no executable home, and AC-1's unit properties are otherwise
   unverifiable on a machine with no systemd. One file, ~130 lines, and it is now the single
   gate for the ops surface. Recorded rather than taken silently.
2. **`restore_drill.sh` gained a `--check` mode** (same reason: m4 needs a runnable check without
   a Postgres). While proving m4, two defects in my own checker were found and fixed:
   the self-test grepped `$SELF`, so its pattern matched its own line and the check could never
   fail; and the first fix then broke on `sed` against a heading that did not exist. The final
   version scans the body only and detects `set -euo pipefail` separately. This is exactly the
   class of defect R-10 exists for: a checker that cannot fail is not a check.
3. **`/x.txt` test case changed to `/x.py`.** The old case asserted `.txt` is refused; the plan
   deliberately allows it. Leaving it would have encoded the new behaviour as a failure.
4. **The load test needed two real bug fixes, both found by running it (R-7):**
   (a) a socket the server refuses never fires `onopen`, so `Promise.all` hung forever; it now
   settles on close/error/timeout; (b) `connectErrors` counted teardown `onerror` events on
   sockets that had opened; it now counts only never-opened sockets. The first run in hosted mode
   also proved the per-IP cap behaves as designed (24 sockets, loopback exempt only when auth is
   off), which is why the documented precondition is a local-mode server.
5. **`ops/check.py` needs a real bash on Windows.** `subprocess` resolved `bash` to WSL's stub
   and failed; it now prefers Git Bash. A tool that only works on the platform it was written on
   is a trap for the next reader.

## Debt

- Created: **DEBT-018** (owner: VPS + Cloudflare tunnel + DNS + units; closes the live-tunnel
  acceptance), **DEBT-019** (owner: first real restore drill on the host).
- Created during APPLY, outside the plan's list: **DEBT-022** (the 1,000-socket run needs several
  source IPs or a raised cap), **DEBT-023** (backup and drill need real Postgres + R2).
- Untouched: DEBT-003/005/010/013/014/015/016/017 (04-02 and 04-03 own them).
- Store after this plan: 15 waiting, 6 done, 0 overdue.

## Skill audit

| Skill | Expected | Invoked | Notes |
|---|---|---|---|
| /architect | plan | yes | at plan time; this plan's ops design came from the arena synthesis (`SYNTHESIS.md` section 7, c2) |
| /security-review (Phase 04 override) | per-task | practical equivalent | the diff is shell, systemd and YAML with no Rust trust-boundary change; the review ran as the `check.py` assertions (hardening flags exact, no literal secrets, restore never targets prod, backup fails closed on a zero-byte dump, tunnel ingress is loopback-only). No Rust surface in this plan, so `/paul-security` was not triggered |
| /technical-writing | optional | no | the README is operational prose in the repo's existing terse style; `ops/README.md` is the runbook rather than a document |
| uxui rows | n/a | n/a | zero HTML, CSS or browser JS in this plan; stated in the plan and true in the diff |

## Learnings

Both candidates written to `.paul/LEARNINGS.md` at this UNIFY (not left as prose here): L-024
(a checker that greps its own file for a pattern its own line contains can never fail) and L-025
(a Python helper on Windows resolves `bash` to the WSL stub unless it prefers Git Bash). L-009
was promoted to validated at hits 3 on this same evidence.

## What this plan does not prove (carried as rows)

- The live tunnel (DEBT-018), the real backup to R2 and the real restore (DEBT-019, DEBT-023),
  the 1,000-socket target (DEBT-022).

## Unrelated tree state

`AGENTS.md` carries an uncommitted 25-line section about `agentgrep` that this session did not
write (it appeared in the tree during the session). It is left untouched; whoever added it owns
it.


## Reflect and RSI

- reflect: deferred to phase close (reflect: pstack). This is plan 1 of 3; the pass runs at the
  UNIFY of 04-03.
- instruction edits proposed: none at this plan. Two learnings were added (L-024, L-025) and one
  existing row was promoted; no reusable instruction needs editing yet.
- rsi: deferred to phase close (reflect: pstack). No experiment candidate from this plan rose
  above the two candidate rows, which are checkable by inspection.

## Close-out writes

- rules: +0 (no new Session Rule; R-4..R-10 unchanged)
- decisions: +0 (no new D-NN; the ops shape came from the approved arena synthesis)
- learnings: +2 added (L-024 self-matching checker, L-025 bash resolution on Windows),
  3 bumped (L-003 3->4, L-009 2->3, L-013 1->2), 1 promoted (L-009 candidate -> validated at
  hits 3, per the file's own rule)
- debt: +5 added (DEBT-018, -019, -022, -023, -024), 0 discharged

## Open risk: UNIFIED is not LANDED

This plan's files are **uncommitted** (git status: `ops/`, `hub/SOURCE.txt`,
`scripts/load_ws.mjs`, `.paul/phases/04-deploy/`, plus edits to `Procfile`, `scripts/dev.sh`,
`crates/hub-core/src/static_files.rs` and the `.paul/` state files). The owner's rule is commit
only when asked. DEBT-024 carries the ask with a check that fails until a commit contains
`ops/cloudflared.yml`.

Also in the tree and not this plan's: `AGENTS.md` carries an uncommitted 25-line `agentgrep`
section that this session did not write.

## Deferred Items

Generated from `.paul/DEBT.toml` (`python ~/.agents/paul-framework/scripts/debt.py --run-checks`:
16 waiting, 0 overdue, 6 done). Rows this plan created or touched:

| Id | Owner | Trigger | Owed |
|---|---|---|---|
| DEBT-018 | owner | phase:04 | VPS + Cloudflare tunnel + DNS + units installed; then the live tunnel check |
| DEBT-019 | owner | event:first-nightly-backup | First real restore drill on the production host |
| DEBT-022 | agent | phase:04 | The 1,000-socket load run (needs several source IPs or a raised cap) |
| DEBT-023 | owner | event:r2-and-postgres-provisioned | Real pg_dump to R2 and a full restore rehearsal |
| DEBT-024 | owner | event:owner-commit | Commit the 04-01 work (UNIFIED is not landed) |

Not this plan's, still open for 04-02 and 04-03: DEBT-003 (lab.js reconnect, 04-02), DEBT-015
(CI-able browser check), DEBT-016 and DEBT-017 (served-page five-gate runner, 04-03), DEBT-014
(owner, link token), plus the owner launch rows DEBT-005, -010, -013.

## Verification caveat

The load test and the mutation runs executed against a **dirty tree** shared with uncommitted
plan work (this plan's own files). That is not contamination from a second plan: no other plan
was in flight, so the tree tested is exactly the tree this plan produced. The one state that
shares mutable infrastructure is the pgsqlite dev database: it was left untouched by this plan
(no schema change, no migration), so nothing was skipped to protect it.

## Next

`/paul:unify .paul/phases/04-deploy/04-01-PLAN.md`, then 04-02 (CSP, rate buckets, TTFC, feed
reserve, DEBT-003 reconnect).
