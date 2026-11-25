---
phase: 06-desktop
plan: 02
status: complete
date: 2026-11-25
---

# 06-02 SUMMARY: the `vtradingview://` hand-off

## What shipped

A deep link that finishes Google sign-in in the browser now reaches the hosted page as one
`CustomEvent`, on all three paths: the app already running (second launch → `single-instance`), the
app not running (cold start → argv → `get_current()`), and the app whose page has not finished loading
(queued, then flushed on load). No navigation, no reload, no lost page state.

| File | Change |
|---|---|
| `crates/desktop/src/deeplink.rs` | New: `parse_code`, `Emitter` trait, `event_js`, `deliver`, `flush` |
| `crates/desktop/src/state.rs` | `pending: Mutex<VecDeque<String>>`, `page_ready`, `needs_rotation`; `begin()` consumes the rotation flag |
| `crates/desktop/src/window.rs` | `on_page_load` moved here (builder-only API): `Started` clears readiness, `Finished` flushes |
| `crates/desktop/src/lib.rs` | `single-instance` → `handle_cli_arguments`, cold-start `get_current()`, `on_open_url`, dev-time `register()` |
| `crates/desktop/tests/deeplink.rs` | The parser contract, including the server's own URL shape |

The capability file is **unchanged** (0 `plugin:`/`core:` permissions): delivery is a Rust-side `eval`,
which needs no ACL entry — D-029 holding exactly as designed.

## Verification, all bare

| Check | Result |
|---|---|
| `cargo +…-msvc test --manifest-path crates/desktop/Cargo.toml` | **31 passed** (24 lib + 5 capability + 2 deeplink), 0 failed |
| `cargo test --workspace` (root) | **139 passed, 1 ignored** — unchanged |
| clippy root + desktop, `-D warnings` | exit 0 both |
| fmt root + desktop | exit 0 both |

## Mutations (R-10)

| # | Mutation | Observed |
|---|---|---|
| m3 | `parse_code` host check relaxed to `host_str().is_none()` | `short_codes_and_wrong_shapes_are_refused` **FAILED**. Restored green. |
| m4 | `page_ready.store(true)` moved **after** the drain | First attempt **passed** — the test was vacuous; see below. After the test was fixed to observe the ordering directly, `a_link_arriving_during_the_drain_is_delivered_not_lost` **FAILED** on `(b) page_ready must already be true when the drain begins`. Restored green. |

**m4's first pass is the finding worth keeping.** My original mid-drain test injected the second link
from inside `eval` and asserted only "it was delivered". It passed under the mutation, because the
queue drain happened to pick up the injected code from `pending` anyway. The test now records
`page_ready` as observed *at the first eval of the drain* — the actual difference between the two
orderings — and that assertion cannot be satisfied by luck. A test whose mechanism cannot distinguish
the two behaviours is a test that certifies nothing.

## Live checks (real window, real OS protocol, CDP)

The protocol was registered by the app itself (`registered vtradingview://`), and the registry pointed
at the dev binary:

```
HKCU\Software\Classes\vtradingview\shell\open\command
  (Default) REG_SZ "…\crates\desktop\target\debug\desktop.exe" "%1"
```

| AC | Observed |
|---|---|
| AC-2 (warm) | After installing a listener in the live page, `cmd start "" "vtradingview://auth?code=abcdefghij_klmnopqrstu"` produced `deep link delivered (22 chars)`; the page held `["abcdefghij_klmnopqrstu"]` and `location.href === <the same hub URL>` → **no reload, no navigation, state intact** |
| AC-3 (queued) | Covered by unit test (`two_queued_links_arrive_in_order_and_only_once`, `a_link_before_the_page_is_ready_is_queued_then_delivered_on_flush`) plus the live cold-start log, which shows the queue path (below) |
| AC-4 (cold) | Launching with the URL in argv logged `deep link queued (21 chars, page not ready)` then, 0.4 s later, `queued deep link delivered (21 chars)` — the exact sequence that silently fails without `get_current()` plus the queue |
| AC-5 (refusal) | `vtradingview://auth?code=short` → `deep link refused (did not parse as vtradingview://auth?code=)`; no event, process still running (1 process, no panic) |

## Two plan corrections found while applying

1. **`on_page_load` is builder-only.** The plan put it in `lib.rs` as `win.on_page_load(...)`; that
   method does not exist on a built window (`tauri-2.12.7/src/webview/webview_window.rs` has it only on
   `WebviewWindowBuilder`). Moved into `window.rs`, which already owns the builder.
2. **`deep_link()` needs the `DeepLinkExt` trait in scope**, and the desktop API is
   `handle_cli_arguments` / `get_current` / `register` / `is_registered` — `on_open_url` exists but does
   not itself parse argv. Using `handle_cli_arguments` in the single-instance callback gives one code
   path for both the second launch and the cold start, instead of two parsers that can drift.

## The live round trip still cannot complete — and it is 06-03's absence, not a defect here

A real Google sign-in from the app currently ends on `/hub.html?signin=failed`. Traced end to end:

```
app window: deployed auth.js (v0.1.1) does location.href = "/auth/google/start?client=web"
  → that 302 sets __Host-vtv_oauth IN THE WEBVIEW JAR   (measured: Secure; HttpOnly; SameSite=Lax)
  → 302 to accounts.google.com
      → the guard refuses in-window and opens that raw URL in the SYSTEM browser
          → the browser has no __Host-vtv_oauth: it never requested /auth/google/start
              → Google signs in, redirects to /auth/google/callback
                  → server: TicketMissing → Landing::Failed   (crates/hub-core/src/auth/mod.rs:741)
                      → "/hub.html?signin=failed"
```

The cookie is written into one jar and read from another. Every guard behaved correctly; the deployed
**page** is what is wrong, and the fix is 06-03's desktop branch: it calls `start_sign_in()`, which opens
`/auth/google/start?client=desktop&dk=…` **in the system browser**, so the sealed OAuth ticket lands in
the jar the callback later reads. 06-03's `if (!isDesktop()) loadGis(...)` also stops Google Identity
Services from auto-navigating the window, which is what triggers the refusal path above on page load.

Independently confirmed: Google's console now accepts the redirect URI. Following the live
`/auth/google/start` 302 lands on the real sign-in page ("Sign in to continue to vtradingview.com"),
so **DEBT-013 is discharged** and should be marked so.

## What 06-03 inherits

- The event is `vtv:desktop-code` with `detail.code` (`crates/desktop/src/deeplink.rs` `EVENT`).
- The code arrives **without** any navigation, so the page's in-memory `desktop_key` survives it.
- The app rotates the key after a delivery (`needs_rotation`), so a failed exchange lands the user on
  "Try again" with a fresh key, never a spent one.
- A deep link that arrives before the page is ready is delivered on `PageLoadEvent::Finished`, which
  means the page's listener must be installed by then — `auth.js` is `defer`, so it is.
