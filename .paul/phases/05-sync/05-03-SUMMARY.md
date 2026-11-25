# 05-03 SUMMARY — hub/sync.js: the client half of synced state

**Plan:** `.paul/phases/05-sync/05-03-PLAN.md` · **Phase:** 05 (Synced state) · **Status:** APPLY
complete, all four tasks, all suites green.

## What shipped

| File | What |
|---|---|
| `hub/sync.js` | The sync layer: Storage wrapper, allow-list, per-key state machine, iframe channel, sign-in merge, notices. Attaches `window.VtvSync`. |
| `hub/hub.html` | `sync.js` as the first script (load-bearing comment), `#syncNotices` region, `.notice.sync-n` + `.keys` CSS. |
| `hub/auth.js` | The avatar-menu status line (`syncLine()`), wired to `VtvSync.on('status')`: saved / saving / offline / conflict, icon plus words every time. |
| `hub/vela.html`, `hub/lightweight.html`, `hub/klinechart.html` | `sync.js` before the first vendor script, same one-line comment. |
| `docs/vela-storage-keys.md` | Q5 answered by measurement. |
| `hub/tests/sync_harness.js` | 13 named cases over CUJ 3/4/5 + AC-8/9/10/12. |
| `.paul/runs/check-0503.mjs` | The served check: 22 cases, `--list`, receipts in `.paul/runs/shots-0503/receipts/`. |

## The allow-list as shipped

Exact: `chartHub`, `chartHubDraft`, `chartHubFavs`, `chartHubIndicators`, `chartHubScripts`,
`chartHubTheme`, `tradexDrawFavs`, `tradexFavBar`. Prefix: `vela:` (placeholder; Vela's workspace
key is never written — `persist: false` at `hub/vela.html:148`). `vtv:ver:<key>` (the version
cache) and `backup:<key>:<UTC YYYY-MM-DD>-<HHMMSS>` never sync: the wrapper's `isExcluded`
defence-in-depth **and** the allow-list itself (EXACT/PREFIXES reject them first — proven by
mutation 2, below).

## The channel vocabulary

- child → parent: `{vtv:1, t:"set", key, value}`, `{vtv:1, t:"del", key}`, `{vtv:1, t:"hello"}`,
  `{vtv:1, t:"need-reload"}`.
- parent → child: `{vtv:1, t:"state", signedIn, versions}`, `{vtv:1, t:"reload"}`.
- Every inbound message: `Origin.from(ev)` → `isSameOrigin(TRUSTED)`, nested fallback
  `Origin.from(ev.origin)` (synthetic events in tests), plus `ev.source === expectedWindow`.
  No `Origin` support → reject (never a string compare). Verified live in Chrome.

## The merge order and its crash-safety argument

1. `GET /api/me/state` for the index.
2. Keys on both sides with different values: **back up first** (`backup:<key>:<stamp>`,
   `If-Match: 0`), **then** adopt the server's value + version. A crash mid-merge loses nothing
   but an extra backup.
3. Server-only keys: written through the original `setItem` (a download must not echo itself).
4. Local-only keys: uploaded with `If-Match: 0`, **skipped when a pending write already owns the
   key** (merge racing the debounce double-PUTs — found by test failure, fixed, see bugs).
5. Equal values: nothing, no backup (AC-7).
6. One iframe reload after the last write (AC-9); one conflict notice per key (AC-4/5/6);
   restore via `VtvSync.restore(backupKey)`.

Sign-in truth: hosted `/auth/me` answers `{"auth": true, "user": null}` when signed out —
`signedIn = !!(s.auth && s.user)`, the convention auth.js's `render()` already uses. A 401 parks
the queue; `HubAuth.onChange` resumes it. `HubAuth.onChange` fires immediately when auth is
already resolved and `me.then` fires right after, so `mergedOnce` flips synchronously (two
racing merges double-PUT every key — found by test failure, fixed).

## Every notice state and how the harness reaches it

| Notice | Trigger used |
|---|---|
| Conflict (`role="alert"`) | B writes with a stale `vtv:ver` after another device advanced the server → 409 |
| Backup (Restore per key) | Sign-in on a browser whose local `chartHub` differs from the account's |
| Offline + Retry | `page.route` aborts `/api/me/state/**`, write, retry budget runs |
| Too large (C7) | Wired from the 413 path; not exercised live (needs a >256 KiB value — the cap is server-enforced, AC-10's harness case covers the offline branch) |
| Status line 4 states | `VtvSync.on('status')` in the avatar menu (saved with time, saving spinner, offline+Retry, conflict amber) |

## Verification (all bare, all run this session)

- `node --check` on all five JS files: OK.
- `node .paul/runs/check-0503.mjs`: **22/22** (server: throwaway pgsqlite on 55495 +
  test-idp hub-server on 8771).
- `node hub/tests/sync_harness.js`: **13/13**.
- `python .paul/runs/gate_all.py --base http://127.0.0.1:8770 --pages hub.html`: **8/8 gates**,
  then `--check-receipts`: OK (receipts refreshed).
- `check-0402.sh --no-reconnect`: **56/56**; with `CHECK_0303=1`: **66/66** (DEBT-025's window
  rule — the 03-03 set passes under the shipped CSP with the new sync.js present).
- `justrust test --workspace`: **139 passed, 0 failed**; `cargo fmt --all --check`: clean;
  `justrust clippy --workspace --all-targets -- -D warnings`: ok.
- `--list` case counts asserted before both suites (L-026); the harness count is now enforced
  in the run body (fails when fewer cases ran than `CASES_EXPECTED`).
- R-9 pages half: the eight design gates ran over `hub.html` (the plan's prescription). The three
  edited engine pages were covered served by `check-0402.sh` (56/56, all four pages loaded after
  the sync.js edits) and by the harness's AC12 engine-page case. A full gate run over
  vela/lightweight/klinechart measured 21 failing design checks — all pre-existing
  hub-native-size findings of the kind the owner accepted for hub.html in 03-03 (red kept:
  "hub-native direction fixed by owner"), not introduced by this plan's one-line script
  insertions. Not receipted into `ds-receipts/current/` (the manifest stays the plan's hub.html
  set); the numbers are recorded here as the deviation note R-9 asks for.

## Mutation records

- **M1 (merge adopts the local value instead of the server's):** `raw.set(k, local)` +
  `setVersion(entry.version)`. Landed → AC-8 "server value adopted as live" **FAIL**
  (`{"key":"ETHUSD","from":"B"}` stayed live) → restored → 22/22 **green**.
- **M2 (as planned: stop excluding `vtv:` from the allow-list): NON-BITING.** `vtv:` keys never
  pass `isAllowed` (EXACT/PREFIXES reject them first), so removing `isExcluded` changes nothing
  observable. Recorded as a plan deviation: the plan's mutation premise was wrong — the
  exclusion is defence-in-depth, not load-bearing for AC-2. AC-2 is still proven, by M2b.
- **M2b (replacement, exercises the version-cache boundary):** route `raw.set` through the
  **wrapped** setter. Every `setVersion` re-queued its key: the version cache landed as a
  pending write, `chartHubTheme`'s stored version advanced one extra step, C's subsequent stale
  write then succeeded (`If-Match: 2` against a server at 2) and **AC-4 FAIL** (no conflict
  notice). Restored → 22/22 **green**. This proves the original-setter rule (`raw` captured
  before the wrapper installs) is load-bearing.

## Spec deviations (deliberate, recorded for UNIFY)

1. **`hub/index.html` excluded** from the sync wire-up: it is a ten-line redirect stub with no
   storage writers (plan Fact; spec item 41 listed it as an engine page — it is not one on this
   branch).
2. **Notices are inline `role="alert"`/`role="status"` blocks under the header, not toasts.**
   The plan's precedence note: the conflict/backup actions exist nowhere else, and the guide
   itself sends "notifications that require immediate attention" to inline banners. The toast
   guide's `popover="manual"` mechanics are not used; its a11y rules (named region, visible-text
   announcement, reduced-motion, 320 px reflow) are.
3. **1 MB → 256 KiB**: the value cap is the server's (D-024, 05-01), not re-stated client-side.
4. **IndexedDB mirror: not built (resolved at UNIFY as D-026).** Correction of this file's own
   first claim: the plan's frontmatter does NOT mention IndexedDB (measured: zero occurrences in
   any 05-sync plan). The phrase exists only in handoff prose (`.paul/HANDOFF-2026-11-25-3.md`,
   marked "treat as a draft"). No task, no AC, and the spec require a second storage engine;
   05-03's SCOPE LIMITS already refuse a durable offline queue (Q7/Q8 open). Recorded as a
   decided-absence in `.paul/DECISIONS.md` D-026.
5. **`hub/auth.js` edit scope**: the status line only (the plan's allowed exception); no sign-in
   logic touched.

## Bugs found and fixed during APPLY (all test-first)

1. Signed-out hosted pages ran the merge (`auth:true, user:null` misread as signed in) → two
   `GET /api/me/state` + 401 noise on every signed-out page load. Fixed to `auth && user`.
2. Double merge from `me.then` + immediate `onChange` → mergedOnce flips synchronously.
3. Merge step 3 raced a pending debounced write (phantom 409 + self-inflicted conflict notice)
   → dirty/inflight/retry keys are skipped by the merge.
4. `on` used before definition (notice wiring) → shorthand defined after `VtvSync`.

Bugs 5-7 were found by the UNIFY reflect panel (3 reviewers + synthesizer, all citations
spot-verified) and fixed with their own harness cases (18→19 named cases, all green):
5. **A15 (must-fix):** the merge adopted the server's value even when the backup PUT failed —
   the plan's contract and the C7 string promise "keep its local value". Reachable via two tabs
   colliding on the same 1-second backup stamp (409 on `If-Match: 0`). Fixed: an unbacked key
   keeps its local value and is reported; never silently overwritten.
6. **A16 (should-fix):** one global `conflictNotice` removed the first key's notice when a second
   key conflicted, leaving that key parked in `conflict` with no UI to resolve it. Fixed:
   per-key notice map (`conflictNotices`).
7. **A17 (should-fix):** `keepMine`'s failure path routed a raced 409 into the retry ladder — six
   retries against a guaranteed 409 and a false "offline". Fixed: a version-conflict in keepMine
   re-enters the conflict emit with the new body.
Two AC-intent readings recorded as D-027 (user write during a conflict re-409s/re-notices) and
D-028 (top-level engine page probes /auth/me once; "no request" = no state request), each with
its own harness case (A18, A19/AC12-engine-page).

## Deferred rows

None new. DEBT-033 (256-key cap corrective) and DEBT-034 (no docs page for the contract) remain
open from 05-02 — `docs/vela-storage-keys.md` answers Q5 only, not DEBT-034's contract page.

## Next-phase readiness

Phase 05's last plan is applied. UNIFY reconciles 05-01, 05-04, 05-02, 05-03; the phase commit
follows UNIFY (everything on `arch/axum-tauri` is uncommitted by design until then).
