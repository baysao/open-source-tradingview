---
description: "04-02 apply record: per-page hashed CSP, four rate buckets, TTFC, feed reserve, reconnect"
type: PlanSummary
plan: .paul/phases/04-deploy/04-02-PLAN.md
status: APPLIED
completed: 2026-11-25
---

# 04-02 SUMMARY: public hardening (CSP, buckets, TTFC, reserve, reconnect)

## What shipped

One coherent diff, exactly the plan's four mechanisms plus the DEBT-003 reconnect:

1. **Per-page hashed CSP** — `static_files::CspSet::scan` reads every top-level `.html` at boot,
   hashes each bare inline `<script>` body (`inline_scripts` byte scan, no parser), and builds
   that page's policy (`policy()`: GIS/Google-Fonts exceptions only where a page uses them,
   `frame-ancestors 'self'` because hub.html frames its own engine pages, `upgrade-insecure-requests`
   on https origins only). `serve` inserts the per-page header; `security_headers` stamps the
   strict default only when no handler set one (insert-if-absent). A page edited after boot serves
   its boot-time policy: the drift is a blocked script, never a silent allow.
2. **Four rate buckets** — `net::{Bucket, RateLimits}`: Api 20/40, Auth 5/10, Admin 5/10,
   Ws 10/30 hosted-only (`bucket_for` returns None for `/ws` when not hosted). `is_limited` is
   deleted; `bucket()` maps prefixes plus `/ws` exact. Values are constants next to the struct,
   no env vars. 429 shape and `Retry-After` unchanged.
3. **TTFC, defined once** — `routes/metrics.rs`: `POST /api/metrics/ttfc` (shared mount, both
   modes) guards itself with `net::post_guard` first, caps the body at 1000 bytes and `ms` at
   120000, refuses unknown engines, records into a fixed 15-bucket histogram (`TTFC_EDGES_MS`,
   saturating adds, std Mutex never across `.await`); `GET /admin/metrics` (hosted mount, behind
   the admin gate) returns `{since, engines: {name: {count, p50_ms, p75_ms, p95_ms}}}`, percentiles
   reported as the upper edge of the bucket. In-memory, window = process lifetime (`since`), no
   persistence (spec line 374 blesses the loss; no new unauthenticated Postgres write path).
4. **Reserved default chart (C14)** — `sources::reserved_key()` = Deriv `frxXAUUSD`, pinned by a
   unit test. `Limits.reserved` set in both modes. `join()` at a full cap: longest-idle eviction
   first (`evict_candidate`, never the reserved key), then for a reserved join only,
   `force_candidate` force-evicts the least-loaded live non-reserved upstream; the sweeper still
   reaps the reserve after grace when nobody views it.
5. **Reconnect with backoff (DEBT-003)** — `hub/lab.js` `live()`: `ws.onclose` reopens with
   exponential backoff (1 s doubling, `Math.min(1000 * 2 ** attempt, 30000)`), reset to 1 s after
   60 s of healthy bars, hud + status-strip countdown, `stop()` cancels a pending retry so a
   symbol switch never leaves two sockets. Protocol unchanged (D-002: the new socket re-sends the
   same subscribe message).
6. **Beacon client** — `hub/ttfc.js` (new, self-guarding against double inclusion) loaded by all
   four pages; the three engine pages call `HubTTFC.send()` at their first live bar; hub.html
   loads it with no `data-engine`, so it never sends.

## Execution note (how this plan was applied)

The plan's code was found **already in the working tree** from the interrupted session of
2026-11-24/25 (Tasks 1-4 implemented, check-0402/mutate-0402 drafted, DEBT-003/015 already marked
done in the store) with no SUMMARY and no recorded evidence. This session executed the remaining
half of the plan — qualification and evidence — and re-ran **every gate and every check fresh**
rather than citing the prior session's outputs (R-10: the checkers were edited after their last
recorded pass):

- `cargo test -p hub-core`: **133 passed**, exit 0
- `cargo fmt --all --check`: exit 0; `cargo clippy -p hub-core --all-targets -- -D warnings`
  default and `--features test-idp`: no issues, exit 0 both
- `cargo test --workspace`: **139 passed**, exit 0; `cargo test -p hub-core --features test-idp`:
  **169 passed**, exit 0 (gate tests enumerate the new routes)
- `node --check hub/ttfc.js hub/lab.js`: exit 0
- `.paul/runs/check-0402.sh` full run (hosted test-idp + local + kill-restart reconnect,
  `CHECK_0303=1`): **60/60 passed**, bare exit 0; the reconnect section killed the hosted server
  mid-session and observed the countdown then bars resuming in the same document (2 -> 3 updates,
  `performance.timeOrigin` unchanged). Log: `.paul/runs/check-0402-final.log`.
- `check-0303.mjs` under the shipped CSP: **66/66 passed** (DEBT-015's re-run).
- Mutations: `.paul/runs/mut-all2.log`, **all eight red-then-green** (run 01:37, after the last
  checker edit at 01:26, so the set is current per R-10).

## Task status

| Task | Status | Qualify |
|------|--------|---------|
| 1 CspSet in static_files.rs | PASS | unit tests green (hash form, per-file vs default selection, attributed-block skip, policy directive table); verified fresh this session |
| 2 RateLimits buckets in net.rs | PASS | bucket table incl. `/websocket` not matching, hosted-only Ws, four-bucket isolation, refill-rate-per-bucket tests green |
| 3 metrics.rs + reserve | PASS | histogram/percentile tests (incl. the derived-Default regression test), reserve force-evict tests, route-enumeration gate tests green under `--features test-idp` |
| 4 ttfc.js + page lines + lab.js | PASS | `node --check` exit 0; browser check observes exactly one beacon per engine page, none from hub.html, bodies `{ms in range, engine}` all 204 |
| 5 checks, mutations, debt, SUMMARY | PASS | check-0402 60/60, check-0303 66/66, m1-m8 red-then-green, SUMMARY written |

## Acceptance criteria

- **AC-1 (CSP):** pass. Zero console CSP violations on all four pages in both modes; sign-in
  through the test-idp mint works under the policy; Vela draws with its Pine worker running; the
  served header equals the per-page policy (`/api/*` carries the strict default); the m1 mutation
  proves a rogue inline block is blocked while other pages still load.
- **AC-2 (buckets):** pass. Bucket tests prove the values, the hosted-only Ws skip, the
  isolation, and the unchanged 429 shape; m3 and m7 mutations prove the Auth bucket is both
  correctly sized and actually enforced at the wire.
- **AC-3 (TTFC):** pass. 204 in both modes, no dedup, every 400/415/413/403 case tested (m5, m6),
  admin read shape with `since` proven in the browser check (signed-out and non-admin 404 via the
  gate), 10,000 posts grow nothing, one beacon per load observed.
- **AC-4 (reserve):** pass. Full-cap tests name the victim; the reserved key never refuses, is
  never an ordinary eviction victim, and is still reaped by the sweeper after grace (m4).
- **AC-5 (reconnect):** pass. Observed in the browser check against a real kill/restart:
  countdown shown, bars resume without a reload in the same document.
- **AC-6 (checks can fail):** pass. m1-m8 each red then restored-green; mutation log
  `.paul/runs/mut-all2.log`.

## Mutations (all red-then-green)

| # | Mutation | Check that bites |
|---|----------|------------------|
| m1 | script-src loses the four sha256 hashes | browser check sees the inline scripts blocked; wire header names no hash |
| m2 | worker-src loses `blob:` | policy-text unit test fails; Vela Pine worker produces no plots |
| m3 | Auth bucket at 500/s | Auth refill-rate test fails |
| m4 | `force_candidate` returns None | reserved-at-full-cap test fails (2 tests) |
| m5 | beacon loses the ms cap | beacon 400 test fails |
| m6 | beacon handler ignores `post_guard` | bad-Origin 403 test fails |
| m7 | Auth bucket never enforced | 429-at-the-wire test fails |
| m8 | `header_for` returns the default for every page | per-page CSP header test fails |

m7 and m8 are additions beyond the plan's m1-m6 (added by the prior session when the checkers
were built); both are red-then-green in the same log.

## Decisions

- **DEBT-015: discharged by wiring.** `check-0402.sh` runs `check-0303.mjs` against the
  VTV_TEST_IDP=1 server it already starts (opt-in `CHECK_0303=1`, status from `PIPESTATUS[0]` so
  `tee` cannot mask a failure); this run recorded 66/66 under the shipped CSP. The CI-headless
  half remains narrowed inside DEBT-017's runner work (04-03). Recorded in the row's check
  (`text_present` on `CHECK_0303=1` in check-0402.sh) and note.
- **DEBT-003: discharged.** The check now names the backoff itself
  (`Math.min(1000 * 2 ** attempt, 30000)`), not the pre-existing `onclose` word; observed, not
  just grepped, by the kill-restart section of check-0402.

## Deviations (auto-fixed, verified)

- **ttfc.js posts with `fetch` keepalive only** (the plan sketched `sendBeacon || fetch`):
  `sendBeacon` sends `text/plain` and would 415 under `post_guard`; one path keeps the guard's
  rules exactly satisfied. Verified: every beacon in the browser check returned 204 with
  `application/json`.
- **No `t0` capture in the pages:** `performance.now()` is navigation-start-relative, so ttfc.js
  measures `Math.round(performance.now())` directly at the first live bar. Equivalent, and it
  leaves the hashed inline blocks smaller.
- **Beacon self-guard moved into ttfc.js** (`if (window.HubTTFC) return`), so a page that loads
  it twice still sends once.

## Deferred rows (view of the store)

- DEBT-016 open: render gates load `file://` only; 04-03's five-gate runner owns the fix.
- DEBT-017 open: single five-gate runner; 04-03 ships it (and inherits DEBT-015's CI half).
- DEBT-024 open (owner): the whole phase surface — 03 close-out, 04-01 and now 04-02 — is
  uncommitted; the owner commits only when asked. The plan's `git status` verification line
  cannot hold until then; the tree carries 04-01's files beside this plan's.
- No new rows opened by this plan.

## Skill audit

| Skill | Expected | Invoked | Notes |
|-------|----------|---------|-------|
| /architect | plan | yes (plan stage) | SYNTHESIS.md arena of 2; c2 sections 2/4/5/6 and c1's ttfc.js adopted |
| /security-review (Phase 04 override) | per-task, Tasks 1-3 | yes | loaded at APPLY; reviewed: hash input completeness (sha256 over the exact block bytes, attributed blocks skipped whole), guard order (post_guard first in the beacon handler), `deny_unknown_fields` on the wire type, no new unauthenticated write path, no secrets in logs, deps already pinned (sha2/base64 in Cargo.lock). No critical/high findings. |
| /paul-security | optional | n/a | guidance row; no auth/parsing surface beyond the beacon, covered by the security-review pass |
| /modern-web-guidance | per-task (Task 4) | yes | guides `security`, `trusted-types` retrieved at plan time 2026-11-24 (see below) |
| /huashu-design, /a11y-audit | per-task | not triggered | no style, control or layout change on any page (plan states this; the gate set is complete per R-9) |
| pstack | - | n/a | `pstack_stage: plan; APPLY ran in-session`; plan `<skills>` poteto-mode/architect rows not loaded per the stage rule |

### Modern Web Guidance

- `security`: applied the hash-not-nonce ladder — pages are boot-scanned static files, so
  `sha256` hashes of the exact inline bodies need no per-response nonce threading; the guide's
  mandatory directives are the policy's directive set (`default-src 'none'`, `object-src 'none'`,
  `base-uri 'none'`, `form-action 'none'`, `frame-ancestors`). Verbatim: "Either the
  'unsafe-inline' keyword, a hash ('sha256-...'), or a nonce ('nonce-...') is required to enable
  inline execution" is the rule the m1 mutation demonstrates in the browser.
- `trusted-types`: retrieved to check `Worker()`; the guide names `new Worker(url)` as a
  ScriptURL sink covered by `worker-src`, so `worker-src 'self' blob:` is the whole requirement;
  no Trusted-Types policy is needed because this plan writes no DOM sink (no `innerHTML`).

## Next-phase readiness

04-03 (pages, docs, spec C2, Python retirement) is the phase's last plan; its DEBT-017 runner now
also inherits DEBT-015's CI-headless half. Launch blockers unchanged: Q2/Q3/Q9, DEBT-018 (host
provisioning), DEBT-005 (first green CI), DEBT-013/010 (real Google round trip).

## Reflect and RSI

- Reflect pass: **deferred to phase close** — 04-02 is the phase's second of three plans;
  `preferences.reflect: pstack` runs once, at the UNIFY of 04-03.
- unslop pass on this SUMMARY (pstack technical-writing rule at UNIFY): no AI-prose patterns found
  (no vocabulary, hedging, sycophancy, filler or rule-of-three tells); em dashes in prose follow
  this repo's summary-family convention and stay.
- Close-out writes: `rules: +0, decisions: +2` (D-022 beacon client, D-023 DEBT-015 wiring);
  `learnings: +1 added (L-031), 1 bumped, 1 promoted (L-023 hits 3), 0 removed`;
  `debt: +0 added, 2 discharged (DEBT-003, DEBT-015), 1 widened (DEBT-024)`.
- Debt store after the close: 0 overdue, 14 waiting, 8 done, 0 open. Pile-up budget (12 open
  non-blocking rows): not exceeded.
- Pre-existing gap, noted not backfilled: STATE's decisions table carries D-019..D-021, but
  DECISIONS.md ends at D-018 (phase 03 recorded those three in STATE only). D-022/D-023 are the
  first rows written to both files.
- **UNIFIED ≠ LANDED:** every file of this plan is uncommitted, together with 04-01's surface and
  the phase 03 close-out (DEBT-024, owner commit, check: `ops/cloudflared.yml` in a commit). The
  ledger and this SUMMARY are bookkeeping, not durability.
