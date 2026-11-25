# Desktop shell: sign-in, permissions, and building (phase 06)

The desktop app is the Tauri crate `crates/desktop` ("TradeX", `com.vtradingview.tradex`). It opens
`https://vtradingview.com/hub.html` in one webview window with a tray (Show and Quit). This page is
what an auditor reads, so exhaustive beats reassuring.

## How the shell signs in (the whole flow)

1. The page's sign-in button calls `window.vtvDesktop.startSignIn()` → `invoke('start_sign_in')` —
   the ONE permission the remote origin holds.
2. Rust mints or rotates the in-memory `desktop_key`, opens the **system browser** at
   `https://vtradingview.com/auth/google/start?client=desktop&dk=<key>`, and returns the key to the
   page. Why the system browser: Google refuses OAuth in embedded webviews.
3. The page shows the waiting overlay (waiting / Try again / Cancel / failed).
4. Google → `/auth/google/callback` → 302 `vtradingview://auth?code=<code>`.
5. Single-instance forwards argv to the running app (or the app cold-starts); the deep-link handler
   validates the scheme and code charset, then reaches the page by a Rust-side `eval` of a
   `CustomEvent` — no permission needed (D-029).
6. The page POSTs `/auth/desktop/exchange` `{code, desktop_key}`:
   - 200 + Set-Cookie → reload → signed in; layouts sync (the phase 05 surface)
   - 401 → expired or reused link: Try again (fresh key)
   - 400 → shell bug: Try again
   - 503 / network → server down / offline
7. A code is single-use with 60 s of life. Any desktop exchange attempt burns the code (D-015).

## What the remote page can do, exhaustively

| Capability | Granted | Proof |
|---|---|---|
| `start_sign_in` command | yes (`allow-start-sign-in`) | AC-3 (invoke succeeds from the page) |
| `plugin:opener|open_url`, `plugin:deep-link|*`, `plugin:fs|*`, `core:event:*` | no | AC-3 negative cases; each must reject |
| navigation to a non-`vtradingview.com` URL | no (guard) | AC-2 unit test + guard mutation |
| `window.open` / `target=_blank` | no (guard) | AC-2 `on_new_window` case |

D-029, stated once: `core:event:default` would be four permissions plus a cross-webview bus to an
untrusted page, and a fragment navigation would reload the page and kill the in-memory
`desktop_key` while the code still has life — so the event arrives by Rust-side `eval` instead.

## Building it

- **Windows needs MSVC.** `rust-toolchain.toml` pins `channel = "1.95.0"`, which on this box
  resolves to the gnu host (`rustc -vV` → `host: x86_64-pc-windows-gnu`); Tauri on Windows needs
  MSVC. Name the toolchain explicitly:
  `cargo +stable-x86_64-pc-windows-msvc tauri build`
- **Linux packages** (the four Tauri v2 names, also installed by `desktop.yml`'s `check-linux`):
  `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`
- **The crate is its own Cargo workspace** (D-030): root `cargo test --workspace` deliberately does
  not build it. Build and test it by manifest:
  `cargo test --manifest-path crates/desktop/Cargo.toml --all-targets`
- **Installers are unsigned beta.** `.github/workflows/desktop.yml` builds NSIS / deb / AppImage on
  a vX.Y.Z tag while Q4 (code signing) is open: no `certificateThumbprint`, no `signCommand`, no
  notarisation keys in `tauri.conf.json`. Adding certificates later is a secrets-only change
  (`WINDOWS_CERTIFICATE`, `WINDOWS_CERTIFICATE_PASSWORD` wired and guarded). The macOS leg joins
  the matrix when Q4 brings a certificate.
