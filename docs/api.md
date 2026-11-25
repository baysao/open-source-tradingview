# `/api/me/state` — the per-user saved-state contract (phase 05, documented from the code)

The wire contract below is transcribed from `crates/hub-core/src/routes/state.rs`'s module doc —
the code is the source, and this page exists because that doc was the only copy of the contract
(DEBT-034). A client author outside this repo can implement against this page; every error string
here appears verbatim in the route handlers.

## The four calls

| Call | Request | Success | Failure |
|---|---|---|---|
| `GET /api/me/state` | — | 200 `{"keys":[{"key","version","updated_at"}]}` | 401 |
| `GET /api/me/state/{key}` | — | 200 `{"key","value","version","updated_at"}` | 401; 404 `not found` |
| `PUT /api/me/state/{key}` | body `{"value":<string>}`, header `If-Match: <n>` | 200 `{"key","version"}` | 409 `{"error":"version conflict","version","value"}`; 413 `value too large`; 409 `too many keys`; 400 `bad key`; 428 `if-match required`; 400 `bad if-match`; 401 |
| `DELETE /api/me/state/{key}` | — | 204 | 401 |

## Rules a client author cannot infer from the table

1. `If-Match: 0` (`expected == 0`) means the key must not exist yet — an insert.
2. Key charset: 1 to 128 bytes of `A-Za-z0-9` or one of `_-.:`. `:` is required because
   `backup:<key>:<date>` keys are written through this same path during a merge; `/` would collide
   with the path segment; whitespace never appears in a written key.
3. The list call omits values on purpose: it is the merge/status snapshot, and shipping every
   value would make it the largest response in the app.
4. A signed-out caller gets 401 before any key check on all four routes.

## Caps, measured

- 256 keys per user, 256 KiB per value (D-024). The key cap is corrective, not preventive: a burst
  of concurrent inserts from one user can transiently hold more than 256 rows (D-031).
