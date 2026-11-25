# Phase 06 architecture: Tauri desktop

Contract for plans 06-01 to 06-04. Two independent design candidates were run (arena, both on the parent
model per `~/.agents/pstack-models.md`'s `inherit-parent`), and this file records the synthesis. Where the
candidates differed, the decision is in "Load-bearing decisions" and the rejected alternative is named so
the next reader does not re-litigate it.

## Problem

The hub is a browser app. Phase 06 ships an installed desktop app that opens the same hosted UI in a
native window, signs in through the *system* browser (Google refuses OAuth inside embedded webviews), and
grants the remote page almost nothing. Every later plugin added to the shell must not become reachable from
that page by accident.

## Shape

```
crates/desktop      own Cargo workspace (NOT a root member)  — see D-030
  Cargo.toml          tauri 2.12, tauri-build 2.7, deep-link 2.6, single-instance 2.5, opener 2.7
  build.rs            tauri_build::try_build(Attributes::new().app_manifest(
                        AppManifest::new().commands(&["start_sign_in"])))   // generates allow-start_sign_in
  tauri.conf.json     productName, identifier, deep-link scheme, tray; NO window here (built in Rust)
  capabilities/hub.json   the ONE capability: windows ["main"], remote urls
                          ["https://vtradingview.com/*"], permissions ["allow-start_sign_in"]
  src/main.rs         plugin order: single-instance -> deep-link -> opener -> setup -> invoke_handler
  src/lib.rs          run(); tray menu ids
  src/window.rs       WebviewWindowBuilder: External url, on_navigation, on_new_window, init script
  src/nav.rs          guard(url) -> bool   (pure, unit-tested: scheme+host EQUALITY)
  src/signin.rs       start_sign_in command; key mint/rotate; opens the system browser
  src/deeplink.rs     on_open_url -> validate -> eval CustomEvent into the ready window, else queue
  src/state.rs        DesktopState { key: Mutex<String>, in_flight: AtomicBool, pending: Mutex<Vec<String>> }
  src/tray.rs         Show / Quit
  src/init.js         include_str!: defines window.vtvDesktop.startSignIn only
  icons/              generated from the existing brand (no new artwork)

hub/auth.js (M)       when window.vtvDesktop exists: startSignIn() + waiting overlay; listens for
                      'vtv:desktop-code' CustomEvent; POSTs /auth/desktop/exchange
```

Unchanged server surface: `POST /auth/desktop/exchange {code, desktop_key}` already ships and is tested
(`crates/hub-core/src/routes/auth.rs:88`, `crates/hub-core/src/routes/table.rs:56`,
`crates/hub-core/tests/auth.rs:424`). Phase 06 adds no route and no migration.

## Load-bearing decisions

1. **The code reaches the page by a Rust-side `eval` of a `CustomEvent`, not by a granted event
   permission and not by a navigation.** The spec's "Desktop shell" paragraph grants the remote origin two
   permissions: the command and the deep-link event. Granting `core:event:default` is four permissions
   (listen/unlisten/emit/emit-to) and hands an untrusted page a cross-webview bus; a navigation
   (`hub.html#dcode=…`) reloads the whole chart app and, worse, defeats the owner's N6 decision, because
   the page's in-memory `desktop_key` is lost on reload while the bound code lives 60 s. Rust therefore
   calls `webview.eval(...)`, which needs no ACL entry, and the page receives
   `new CustomEvent('vtv:desktop-code', {detail: {code}})`. Result: **exactly one** permission for the
   remote origin, one fewer than the spec demands. Recorded as D-029; the spec paragraph is amended in
   06-04's docs task.
   *Rejected:* `core:event:default` (wider surface, semantics to argue instead of a count); fragment
   navigation (reload kills the key, code enters history).

2. **`crates/desktop` is its own Cargo workspace, not a root member.** `crates/hub-types/Cargo.toml:3` uses
   `edition.workspace = true`, so a second workspace cannot path-depend on it; and the spec's C22 rationale
   ("desktop depends on hub-types for the origin and scheme constants") does not buy drift protection —
   the deployed origin comes from `/etc/vtv/env`, so a compile-time const cannot detect a mismatch. The
   three constants (`PUBLIC_ORIGIN`, `DEEP_LINK_SCHEME`, `HUB_PATH`) live in `crates/desktop/src/config.rs`
   instead. Consequence, and the reason this is safe: the root `cargo test --workspace` and clippy never
   see tauri code, so the 139-test gate and the gnu-pinned host toolchain are untouched; the desktop crate
   is gated by its own CI job. Recorded as D-030.
   *Rejected:* root member + `default-members` (bare `cargo test` on this Windows box would then try to
   build tauri with the gnu toolchain and fail); root member + `.cargo/config.toml [build] target` (breaks
   the other three crates' local build, and the msvc target is not installed for the pinned toolchain).

3. **The page surface is one injected function, not `withGlobalTauri: true`.** `withGlobalTauri` publishes
   the whole `__TAURI__` API into a remote origin, where a future capability typo becomes a live
   privilege. Instead `src/init.js` (an initialization script) defines only
   `window.vtvDesktop = { startSignIn }`, and `startSignIn` reaches
   `window.__TAURI_INTERNALS__.invoke('start_sign_in')`. The ACL still gates the invoke, so this is
   defence in depth: a capability mistake leaves the page unable to call anything but the one command.
   Deviation from spec item 46's "`window.__TAURI__` present" test; recorded in 06-04's docs task.

4. **The key is minted lazily in Rust, stays in Rust, and is handed to the page by the one command.**
   `DesktopState.key: Mutex<String>`; `start_sign_in` mints on first use, reuses while an un-consumed code
   may still exist, and **rotates after** a code has been delivered (so "Try again" after a failed
   exchange gets a fresh key, while a second click 2 s later reuses the live one instead of stranding it).
   `in_flight` CAS makes a double-invoke return the same key without reopening the browser. The key is
   never logged (the `Refusal` discipline from `crates/hub-core/src/auth/mod.rs:6`).

5. **The exchange stays a page-side `fetch`.** The request is same-origin
   (`https://vtradingview.com/auth/desktop/exchange` from a page at that origin), so it sends
   `Origin: https://vtradingview.com`, which is exactly what D-010's CSRF guard requires, and `Set-Cookie`
   lands in the webview's own cookie jar for free. Rust-side HTTP would need a second cookie jar and
   manual re-injection.

## Sign-in sequence (the whole flow, once)

```
click "Sign in with Google" in the app window
  └─ window.vtvDesktop.startSignIn()
       └─ invoke('start_sign_in')            [the ONE granted permission]
            ├─ mint key if none / rotate if a code was delivered
            ├─ opener: open https://vtradingview.com/auth/google/start?client=desktop&dk=<key>
            └─ return key  ────────────────────▶ page keeps it in memory
  page shows the waiting overlay (mockup 06: waiting / Try again / Cancel / failed)
Google (system browser) -> /auth/google/callback -> 302 vtradingview://auth?code=<code>
  └─ single-instance forwards argv to the running app (or the app cold-starts)
       └─ deep_link on_open_url -> validate scheme+code charset -> eval CustomEvent
            └─ page: POST /auth/desktop/exchange {code, desktop_key}
                 ├─ 200 + Set-Cookie -> reload() -> signed in, layouts sync (phase 05 surface)
                 ├─ 401 "sign-in failed"  -> expired/reused: Try again (fresh key)
                 ├─ 400 "bad request"     -> shell bug: Try again
                 └─ 503 / network         -> server down / offline copy
```

## What the remote page can do, exhaustively

| Capability | Granted | Test that proves the boundary |
|---|---|---|
| `start_sign_in` command | yes | AC-3 (invoke succeeds from the page) |
| `plugin:opener|open_url`, `plugin:deep-link|*`, `plugin:fs|*`, `core:event:*` | no | AC-3 negative cases; each must reject |
| navigation to a non-`vtradingview.com` URL | no (guard) | AC-2 unit test + guard mutation |
| `window.open` / `target=_blank` | no (guard) | AC-2 `on_new_window` case |

## Toolchain, measured

- `rust-toolchain.toml` pins `channel = "1.95.0"`, which resolves to the **gnu** host on this box
  (`rustc -vV` → `host: x86_64-pc-windows-gnu`). Tauri on Windows needs MSVC.
- `rustup toolchain list` also holds `stable-x86_64-pc-windows-msvc` (the rustup default) and
  `x86_64-pc-windows-msvc` is an installed target for it. Every Windows desktop build therefore names it:
  `cargo +stable-x86_64-pc-windows-msvc tauri build`. No `.cargo/config.toml` and no nested
  `rust-toolchain.toml` (a platform-specific channel string in a toolchain file breaks the Linux CI job).
- `cargo tauri info` on this box: tauri-cli 2.12.6, WebView2 158.0.4370.33, MSVC Build Tools 2026 and
  VS Community 2022 both present.
- Stable 2.x plugin versions exist and are used (no alpha): `tauri 2.12.7`, `tauri-build 2.7.6`,
  `tauri-plugin-deep-link 2.6.3`, `tauri-plugin-single-instance 2.5.5`, `tauri-plugin-opener 2.7.3`.

## Open items this plan set does NOT close

- **Q4 signing certificates** are not in hand, so the bundle ships unsigned betas. Pre-registered as an
  owner debt row in 06-04 so APPLY never blocks on a certificate.
- **Visual judgement of a native window** cannot be made by obscura (it drives a browser, not a window).
  Each plan that renders the shell leaves one residual owner `uat` row per R-6; the agent runs every
  assertion it can (process starts, logged load URL, guard unit tests, in-page checks over http).
