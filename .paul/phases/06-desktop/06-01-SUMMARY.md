---
phase: 06-desktop
plan: 01
status: complete
date: 2026-11-25
---

# 06-01 SUMMARY: the desktop shell, its capability, and the guard

## What shipped

`crates/desktop` is a Tauri 2 app that opens `https://vtradingview.com/hub.html` in one native window,
grants the remote page exactly one permission, refuses every navigation off-origin, exposes one injected
JS function, and has a tray with Show and Quit.

| File | Role |
|---|---|
| `crates/desktop/Cargo.toml` | Own `[workspace]` (D-030); tauri 2.12 resolved to **2.12.7**, `tauri-build` → **2.7.6**, `deep-link` → **2.6.3**, `single-instance` → **2.5.5**, `opener` → **2.7.3** — all stable 2.x, no alphas |
| `crates/desktop/build.rs` | `try_build` + `AppManifest::commands(["start_sign_in"])` |
| `crates/desktop/tauri.conf.json` | `withGlobalTauri: false`, `windows: []` (built in Rust), deep-link scheme, tray, installers |
| `crates/desktop/capabilities/hub.json` | One capability: `remote.urls ["https://vtradingview.com/*"]`, `local: false`, `permissions ["allow-start-sign-in"]` |
| `crates/desktop/src/config.rs` | `PUBLIC_ORIGIN`, `DEEP_LINK_SCHEME`, `HUB_PATH`, `KEY_BYTES`, `start_url()` |
| `crates/desktop/src/nav.rs` | `is_allowed()` — scheme + host + port **equality** |
| `crates/desktop/src/state.rs` | `DesktopState` (key, last-start), `should_reopen()` debounce, `mint_key()` |
| `crates/desktop/src/signin.rs` | The one command; browser launch on a worker thread |
| `crates/desktop/src/window.rs` | The window, `on_navigation`, `on_new_window`, close-to-hide |
| `crates/desktop/src/tray.rs` | Show and Quit; left-click raises |
| `crates/desktop/src/init.js` | The whole page surface: frozen `window.vtvDesktop.startSignIn` |
| `crates/desktop/tests/capability.rs` | Five manifest assertions that fail when the grant widens |
| `crates/desktop/icons/` | Generated from the hub's own tokens (`accent #e3a33a` on `bg #151412`) — no new artwork |
| `.gitignore` | `crates/desktop/gen/` |

## Verification, all run bare

| Check | Result |
|---|---|
| `cargo +stable-x86_64-pc-windows-msvc test --manifest-path crates/desktop/Cargo.toml` | **16 passed**, 0 failed (11 lib + 5 capability) |
| `cargo test --workspace` (root) | **139 passed, 1 ignored** — the phase-05 baseline, unchanged; no tauri crate built (AC-5) |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo +…-msvc clippy --manifest-path crates/desktop/Cargo.toml --all-targets -- -D warnings` | exit 0 |
| `cargo fmt --all --check` / desktop crate fmt | exit 0 both |
| Generated ACL | `grep -c "allow-start-sign-in" crates/desktop/gen/schemas/acl-manifests.json` → 1; `capabilities.json` shows the resolved grant |

## Mutations (R-10), each observed red then restored green

| # | Mutation | Observed |
|---|---|---|
| m1 | `nav.rs` host equality → `url.as_str().starts_with("https://vtradingview.com")` | `a_prefix_spoofed_host_is_refused` FAILED on `https://vtradingview.com.evil.com/hub.html`, and `a_non_default_port_is_refused` failed too. Restored, 11/11 green. |
| m2 | capability `permissions` + `"core:event:default"` | `the_remote_page_gets_exactly_one_permission` FAILED ("a superset is the defect this test exists for") **and** `no_plugin_or_core_permission_reaches_the_remote_origin` FAILED. Restored, 5/5 green. |

## Live checks (agent-run, real window, CDP over WebView2)

The app was run against the live origin (`/api/health` 200) with
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=…`, and driven with `Runtime.evaluate`.

| AC | Observed |
|---|---|
| AC-1 | Log: `opening https://vtradingview.com/hub.html`; the page target reported exactly that URL |
| AC-2 (in production) | The guard refused `https://accounts.google.com/o/oauth2/v2/auth?…` in-window (`refusing in-window navigation to …`) and handed it to the system browser |
| AC-3 | Surface: `typeof = object`, `keys = startSignIn`, `frozen = true`. `startSignIn()` **RESOLVED len=43 base64url=true in 0.00 s**; the browser hand-off logged 0.23 s later. Debounce: two awaited calls returned `{"lenA":43,"lenB":43,"same":true}` — one key, no second tab. ACL: `plugin:opener|open_url`, `plugin:fs|read_text_file`, `plugin:deep-link|get_current`, `core:event|listen` all **REJECTED**, with messages naming the permission each would have needed |
| AC-4 | Tray built with Show/Quit ids; close-to-hide wired via `CloseRequested` + `prevent_close`. **Not clicked by a human** — that is the residual `uat` row 06-04 registers |

## Three defects the live check found, none reachable from the unit tests

1. **The permission id was wrong in the plan.** The plan's `allow-start_sign_in` does not exist.
   `tauri-utils-2.10.4/src/acl/build.rs:331` does `command.replace('_', "-")`, so the generated id is
   **`allow-start-sign-in`**. The build failed loudly on the plan's spelling; fixed in
   `capabilities/hub.json` and `tests/capability.rs`.
2. **`start_sign_in` was dead on arrival: "state not managed for field `st`".** `DesktopState` was
   constructed and passed to the window builder but never `app.manage()`d, so every call failed before
   reaching the key mint. Found only by the live invoke. Fixed by managing it in `setup`; `window::build`
   now takes no state.
3. **The browser launch froze the whole app for 84 s.** A synchronous `open_url` on a Tauri command runs
   on the main thread; measured: 84 s blocked, then `os error 1223` (ERROR_CANCELLED). Fixed by launching
   on a worker thread (`signin.rs`), so the command returns the key immediately — measured 0.00 s — and
   the page's own 60 s overlay timeout remains the failure path if no code ever arrives.

## Deviation from the plan, recorded

- `Image::from_bytes` does not exist in this tauri build (it is feature-gated); `tauri::include_image!`
  is the supported path and resolves relative to the **crate root**, not `src/`. Both corrected, and the
  icon path is now `icons/32x32.png`.
- `window::build` signature changed from `(app, state)` to `(app)` as a consequence of defect 2. 06-02
  must read the state via `app.state::<DesktopState>()` rather than a parameter.

## Blocked, not by this plan: the live Google round trip

The desktop flow reaches Google correctly, and Google refuses with
**`Error 400: redirect_uri_mismatch`**. That is **DEBT-013** (`owner`, still `open`): the OAuth client has
not had its redirect URI registered. Probed live:
`GET /auth/google/start?client=desktop&dk=…` → 302 with
`redirect_uri=https%3A%2F%2Fvtradingview.com%2Fauth%2Fgoogle%2Fcallback`, which the client does not allow.
It blocks the browser path identically, so it is not a desktop defect — and it must be discharged before
06-04's human-verify checkpoint can pass step 4.

## What 06-02 inherits

- Read the key state with `app.state::<crate::state::DesktopState>()`; do not thread it through `build`.
- The window is built in `window::build(app)`; `on_page_load` is available on the returned handle.
- The command returns before the browser opens, so nothing may assume the tab exists.
