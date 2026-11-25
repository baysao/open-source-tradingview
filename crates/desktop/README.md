# The TradeX desktop shell

A Tauri 2 app that opens `https://vtradingview.com/hub.html` in a native window. It is the phase-06
deliverable; the contract for the whole phase is `.paul/phases/06-desktop/06-ARCHITECTURE.md`.

## This crate is its own Cargo workspace

`crates/desktop` is **not** a member of the root workspace (decision D-030). Two consequences:

- `cargo test --workspace` and `cargo clippy --workspace` at the repo root **do not build this crate**.
  They keep their phase-05 meaning and their gnu-pinned host toolchain.
- This crate therefore needs its own CI job (`.github/workflows/desktop.yml`, added in 06-04). A crate
  nothing builds is a crate that is already broken.

Why: a second workspace cannot path-depend on a crate that uses `edition.workspace = true`
(`crates/hub-types/Cargo.toml:3`). The spec's C22 wanted the origin constants shared through
`hub-types`; they live in `src/config.rs` instead, and nothing is lost, because the deployed origin
comes from `/etc/vtv/env` and a compile-time constant can never detect a mismatch.

## Building it: MSVC on Windows

`rust-toolchain.toml` pins `channel = "1.95.0"`, which resolves to the **gnu** host on a Windows box
(`rustc -vV` reports `host: x86_64-pc-windows-gnu`). Tauri cannot build on gnu Windows, so name the
toolchain on every build:

```sh
cargo +stable-x86_64-pc-windows-msvc test
cargo +stable-x86_64-pc-windows-msvc tauri build
```

Measured on the dev box: tauri-cli 2.12.6, WebView2 158.0.4370.33, MSVC Build Tools 2026 and Visual
Studio 2022 both present, `x86_64-pc-windows-msvc` installed for the stable toolchain.

There is deliberately no `.cargo/config.toml` target override and no nested `rust-toolchain.toml`:
both would break the other three crates locally, or the Linux CI job, respectively.

## Building it: Linux

The system packages Tauri 2 needs:

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

## What the app is allowed to do

One command reaches the hosted page (`allow-start_sign_in`), and nothing else — no filesystem, shell,
HTTP, clipboard, opener or event bus. `src/nav.rs` keeps the window on `https://vtradingview.com`;
every other URL opens in the system browser. `src/init.js` is the page's whole surface:

```js
window.vtvDesktop.startSignIn()   // -> desktop_key, 43 chars
```

`tests/capability.rs` asserts the capability file stays that small; the mutation note in it says what
to look for if a future task widens the list.

## Layout

| Path | Role |
|---|---|
| `src/config.rs` | Public origin, deep-link scheme, hub path, `start_url()` |
| `src/nav.rs` | The navigation guard (pure, unit-tested) |
| `src/state.rs` | The in-memory key and the reopen debounce |
| `src/signin.rs` | The one command |
| `src/window.rs` | The window, both navigation hooks, the close-to-hide behaviour |
| `src/tray.rs` | Show and Quit |
| `src/init.js` | The injected page surface |
| `capabilities/hub.json` | The permission grant, remote-scoped |
