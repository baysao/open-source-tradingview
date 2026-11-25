# Phase 05 architecture: synced state

Contract for plans 05-01 to 05-03. Two independent design candidates were run (arena, both on the parent
model per `~/.agents/pstack-models.md`'s `inherit-parent`), and this file records the synthesis. Where the
candidates differed, the decision is in "Load-bearing decisions" and the rejected alternative is named so
the next reader does not re-litigate it.

## Problem

A user's chart layout, drawings, indicators, scripts, theme and favourites live in `localStorage` on
whatever browser they used. Phase 05 makes them follow the account, without ever destroying one side's
work silently (CUJ 3, CUJ 4, CUJ 5), and without letting one tab's write fight another's.

## Shape

```
hub.html (parent, the only writer)
  ├── sync.js          Storage.prototype wrapper, queue, state machine, merge
  │     └── fetch ──▶  /api/me/state          GET index
  │                    /api/me/state/{key}    GET | PUT(If-Match) | DELETE
  └── <iframe> vela.html / lightweight.html / klinechart.html
        └── sync.js   same file, child role: wrapper + postMessage only, zero fetch

server
  routes/state.rs      4 handlers, thin: validate → Auth::state_* → render
  auth/mod.rs          state_all / state_put / state_delete / expire_backups
  auth/store.rs        S15..S21  (the one statement home)
  migrations/0002      user_state + one index
  lib.rs               spawn_background: 24 h prune beside the hourly session sweep
```

Local mode (`auth: None`, no database): nothing mounts, `sync.js` sees `{"auth": false}` and stays inert.

## Data structures

```rust
// migrations/0002_user_state.up.sql
user_state(
  user_id    uuid        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  key        text        NOT NULL,
  value      text        NOT NULL,       -- the page's string, verbatim
  version    integer     NOT NULL,
  updated_at timestamptz NOT NULL,
  PRIMARY KEY (user_id, key)
)
CREATE INDEX user_state_updated_at ON user_state (updated_at)   -- the prune only

// auth/mod.rs
pub const VALUE_MAX: usize = 262_144;  // the gate's 300_000-byte body cap is the real ceiling (D-024)
pub const KEY_MAX: usize = 256;

pub struct StateRow { key: String, value: String, version: i64, updated_at: DateTime<Utc> }

pub enum PutOutcome {
    Stored { version: i64 },
    Conflict { version: i64, value: String },   // version 0 = the key does not exist
    TooLarge,
    TooManyKeys,
}
```

## Signatures

```rust
impl Auth {
    pub async fn state_all(&self, member: &Member) -> Result<Vec<StateRow>, AuthError>;
    pub async fn state_put(&self, member: &Member, key: &str, value: &str,
                           expected: i64) -> Result<PutOutcome, AuthError>;
    pub async fn state_delete(&self, member: &Member, key: &str) -> Result<(), AuthError>;
    pub async fn expire_backups(&self, cutoff: DateTime<Utc>) -> Result<u64, AuthError>;
}

// routes/state.rs
async fn list(State<AppState>, Extension<Viewer>) -> Response;
async fn get_one(State<AppState>, Extension<Viewer>, Path<String>) -> Response;
async fn put(State<AppState>, Extension<Viewer>, Path<String>, HeaderMap, Json<PutBody>) -> Response;
async fn delete(State<AppState>, Extension<Viewer>, Path<String>) -> Response;
fn valid_key(&str) -> bool;   // 1..=128 bytes of [A-Za-z0-9_.:-]
```

## Statements

```text
S15 SELECT key, value, version, updated_at FROM user_state WHERE user_id = $1 ORDER BY key
S16 SELECT value, version FROM user_state WHERE user_id = $1 AND key = $2
S17 INSERT INTO user_state (user_id, key, value, version, updated_at) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (user_id, key) DO NOTHING
S18 UPDATE user_state SET value = $1, version = $2, updated_at = $3 WHERE user_id = $4 AND key = $5 AND version = $6
S19 DELETE FROM user_state WHERE user_id = $1 AND key = $2
S20 DELETE FROM user_state WHERE user_id = $1 AND key = $2 AND version = $3
S21 DELETE FROM user_state WHERE updated_at < $1 AND key >= $2 AND key < $3   -- 'backup:' .. 'backup;'
```

Placeholders ascend textually in every statement (DEBT-012: pgsqlite mis-maps out-of-order numbers).
Every statement names one table and uses no `RETURNING`, join, subquery, cast, function or default
(D-014); `db.rs::sql_shape_violations` enforces it, and `STATEMENTS` grows 14 → 21 so the guard covers
the new ones.

## The put algorithm, and why each step is there

1. `value.len() > VALUE_MAX` → `TooLarge`, before any statement.
2. Read `S16`.
   - **absent, expected ≠ 0** → `Conflict { version: 0, value: "" }`: the client's cache is stale but
     there is nothing to show it. `0` means "does not exist", stated in the 409 body's contract.
   - **absent, expected = 0** → `S17`. `rows_affected == 1` → version 1. `rows_affected == 0` → a racer
     inserted first: re-read and return its conflict.
   - **present, expected ≠ version** → conflict without writing.
   - **present, expected == version** → `S18` with `current + 1` computed in Rust (an SQL
     `version + 1` is an expression, which D-014 forbids). **Re-read rather than trust
     `rows_affected`**: pgsqlite reports 0 for an unchanged row, so 0 is not proof of a lost race —
     03-02 hit this on `S4`/`S8`.
3. Cap, only on the paths that inserted: list `S15`, count in Rust (`count(*)` is a function). Over
   `KEY_MAX` → undo with `S20` bound to the version this call wrote, then `TooManyKeys`. The undo can only
   remove this call's own row, so a concurrent pair never deletes a legitimate one.

## Client

Public surface, deliberately small:

```js
window.VtvSync = { ready, signedIn, version(key), on(event, fn), restore(backupKey), retry() }
```

- **Allow-list**: eight exact keys plus the `vela:` prefix placeholder (see
  `docs/vela-storage-keys.md`, which answers Q5 by measurement: `hub/vela.html:148` passes `persist:
  false`, so no Vela workspace key is written today). `vtv:` and `backup:` are never synced.
- **One writer**: parent only. Child wraps storage and posts; it makes no state request at all.
- **Channel**: `{vtv:1,t:…}` in both directions, validated per `validate-origins` — `Origin.from(event)`
  with a `try/catch` fallback to `Origin.from(event.origin)`, plus an `event.source` identity check.
- **State per key**: `idle → dirty → 2 s debounce → inflight → idle | conflict | retry`. Retry 1/2/4/8/15 s
  ±20 %, six attempts, then `unsynced` until the next write or `online`. 409 is a decision, never retried;
  401 parks the queue.
- **Merge on sign-in**: index → back up every differing intersection key to
  `backup:<key>:<YYYY-MM-DD>-<HHMMSS>` → **then** adopt the server value → upload local-only keys with
  `If-Match: 0` → one iframe reload if a replaced key is on screen. Back up before adopting, so a crash
  mid-merge costs an extra backup, never a value.

## Load-bearing decisions

| # | Decision | Rejected alternative, and why |
|---|---|---|
| 1 | `VALUE_MAX = 262_144`, named for the measured cap | the spec's 1 MB: the gate refuses any declared body ≥ 300,000 bytes before the handler, so 1 MB is unreachable; a per-route gate exemption is the restructuring this phase forbids |
| 2 | Integer `version` column with compare-and-swap | content hash / digest ETag: costs a read plus a full comparison per write to save four bytes, and pgsqlite's `rows_affected` cannot tell an equal payload from a lost race |
| 3 | Statements in `auth/store.rs` as `S15..S21` | a new `state.rs` statement module: `user_state` is account data and the "one home + `STATEMENTS` shape test" is what keeps the SQL discipline auditable; a second home is a second thing to forget |
| 4 | Corrective cap: insert, count, delete own row on overflow | count-before-insert (two statements, still racy), a lock, or a counter column: more machinery than a number that self-heals on the next write deserves |
| 5 | Per-key `PUT`, no batch endpoint | batch: `sync.js` debounces per key, so a batch saves nothing at steady state and adds all-or-nothing semantics plus a per-item 409 body |
| 6 | Merge in JS, server stays dumb | server-side merge: needs an upsert loop the SQL discipline forbids, and moves the one interesting algorithm out of the code a browser harness can drive |
| 7 | Backups named `backup:<key>:<date>-<time>`, client-chosen | a sequence column (schema beyond `user_state`) or a server-side probe loop (extra statements + subquery-free retry) |
| 8 | Notices are inline `role="alert"`, not toasts | `persistent-toast-notifications`'s `popover="manual"`: its own "immediate attention → inline banner" rule applies, the D4 mockup specifies a banner, and the actions have no other permanent home |
| 9 | `sync.js` not added to `hub/index.html` (D-025) | spec item 41's page list: `index.html` is a 10-line redirect stub with no writers, so a wrapper there syncs nothing |
| 10 | Fix `tests/gate.rs`'s URL construction for templated paths | making `RouteSpec` carry a sample value: one test's need does not belong in the production type |

## Scratch in the code, not in behaviour

Two places where the plan accepts a deliberate ceiling, marked for the next reader:

- `ponytail: cap is corrective, not preventive` — brief overshoot past 256 keys under concurrency,
  self-healing. Upgrade path: a serialising transaction if pgsqlite can honour one.
- `ponytail: vela: prefix is a placeholder` — the key never occurs today (`persist: false`). Upgrade
  path: turn persistence on and the key lands in the allow-list with no plan change.

## Open questions and risks, each with the default taken

| Id | Risk | Default taken | Reopened by |
|---|---|---|---|
| Q5 | Vela's real keys | answered by measurement: none written; `vela:` kept as a placeholder | a Vela option that enables persistence |
| Q7 | total per-user byte cap | not enforced; only per-value and per-key limits | the owner setting a number |
| R16 | the two `tradex*` keys | both allow-listed exactly, no wildcard | a third key appearing |
| R23 | `removeItem` has no server delete | `DELETE /api/me/state/{key}`, idempotent | — |
| R23 | backups count toward 256 | they do (spec), and the merge names a key it could not back up | a quota decision |
| v01 | a value can change byte-for-byte on a round trip | never parse or re-serialise a synced value | — |

## Plan boundaries

- 05-01: storage only. No route, no job caller, no client.
- 05-02: HTTP only. Consumes 05-01's statements unedited; the prune gets a scheduler here.
- 05-03: client only. Consumes 05-02's contract table; no server change.
- 05-04: page gates and the debt store. Wave 1, because 05-03's gate evidence is impossible without
  `hub.html` under the gate set.

Cross-plan dependencies, each naming what it consumes: `05-02 depends_on 05-01: the user_state table,
statements S15..S21, and Auth::{state_all,state_put,state_delete}`; `05-03 depends_on 05-02: the exact
JSON shapes and error strings the client codes against`. `05-04` depends on nothing and must finish before
05-03's Task 4 runs.
