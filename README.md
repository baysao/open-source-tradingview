# vtradingview: hosted TradeX on Rust

A hosted, multi-user TradeX at `https://vtradingview.com/hub.html`: an Axum server replaces the Python
hub, Google sign-in saves each user's work, and a Tauri 2 app shows the same site on the desktop.

> Not affiliated with, endorsed by, or connected to TradingView, Inc. "TradingView" is their trademark.
> This project is an independent open-source tool. It is not financial advice.

> **Status: phases 1 to 3 complete on branch `arch/axum-tauri`; phase 4 (public deploy) in flight.**
> The Python hub is retired on this branch: the Rust server is the only server (see
> [Deploy](#deploy)). Remaining design-review findings are tracked as launch questions
> (see [Open Questions](#open-questions)).

*   **Type:** Application
*   **Stack:** Rust (Axum 0.8, tokio, sqlx with the Postgres driver) + the existing vanilla JS frontend
    (Vela, Lightweight Charts, KLineChart) + pgsqlite over a SQLite file by default, real Postgres by
    `DATABASE_URL` + Tauri 2 desktop shell + mise tasks (dev) and systemd (prod) + Cloudflare Tunnel
*   **Skill Loadout:** PAUL (required), AEGIS (post-build audit), `/seed design` (new screens),
    security-review (auth, SSRF), postgres-pro (pgsqlite parity)
*   **Quality Gates:** Rust-only contract suite, order-flow golden parity, CI on pgsqlite and
    Postgres 17, cross-user authz tests, SSRF suite, design gates through
    `.paul/runs/gate_all.py`, p75 time to first live candle under 2 s
*   **Design doc (single source):** [.paul/design/specs/vtradingview.md](.paul/design/specs/vtradingview.md)
*   **Full plan:** [.paul/PLANNING.md](.paul/PLANNING.md)

## Overview

TradeX today needs Python 3, a virtual environment, numpy, pandas and pyarrow, and a terminal; whatever a
user builds lives in one browser's `localStorage`. This project makes it a public website, with charts
open to anyone and saved state (layouts, drawings, Pine scripts, favourites, indicators, theme) synced to
a Google account, plus a desktop app that shows the same site.

The existing chart UI stays. Seven new screens are added in the hub's own theme: the sign-in control and
avatar menu, sync notices, MT5 bridges settings, delete account, the admin log panel, the desktop
sign-in waiting state, and a privacy page. Mockups: `.paul/design/opendesign/vtradingview/`.

## Architecture

```text
 browser / Tauri window                    vtradingview host (Linux, systemd)
 hub.html + sync.js + auth.js   HTTPS/WSS  cloudflared  ->  127.0.0.1:8770
 iframe: Vela / lw / kline   <-Cloudflare-> hub-server (Axum, one port)
                                            static allow-list | /api/* | /ws | /auth/* | /api/me/* | /admin/*
                                            FeedHub (shared upstreams) | order flow | MT5 bridge client
                                            sqlx -> pgsqlite :55432 (SQLite file) or managed Postgres
```

| Crate | Role |
|---|---|
| `crates/hub-types` | API and websocket message types (the contract) |
| `crates/hub-core` | the single router and all logic (`hub_core::app`) |
| `crates/hub-server` | binary: config, migrations, binds `127.0.0.1:8770` |
| `crates/desktop` | Tauri 2 shell loading the hosted page |

## Data Model

| Table | Key fields |
|---|---|
| `users` | id (uuid), email, email_verified, name, avatar_url, created_at, last_login_at |
| `identities` | user_id, provider `google`, subject (Google `sub`); unique (provider, subject) |
| `sessions` | hashed id, user_id, expires_at (30-day sliding), revoked_at |
| `user_state` | user_id, key, value (JSON as text), version; primary key (user_id, key) |
| `mt5_bridges` | user_id, label, https url, encrypted token, last_ok_at, last_error |
| `desktop_auth_codes` | hashed one-time code, 60 s expiry, used_at |

## API Surface

| Group | Auth | Purpose |
|---|---|---|
| `/hub.html` and static allow-list | public | frontend; everything else 404 |
| `/api/status`, `/api/health`, `/api/symbols`, `/api/instruments`, `/api/bars`, `/api/orderflow/*` | public, rate-limited | existing contract |
| `/ws` | public, session attached when signed in | live data, replaces port 8765 |
| `/auth/google/*`, `/auth/desktop/*`, `/auth/me`, `/auth/logout` | public or session | Google sign-in, desktop handoff |
| `/api/me/state/:key`, `/api/me`, `/api/me/mt5*` | session | synced state, account deletion, MT5 bridges |
| `/admin/logs`, `/admin/restart`, `/admin/metrics` | admin (`ADMIN_EMAILS`) | log panel, feed restart, metrics; 404 for others |

## Deploy

One port: HTTP and `/ws` on 8770, loopback only. Dev stack: `mise run dev` (or `dev:db`,
`dev:hub`, `dev:tunnel`; each calls `scripts/dev.sh _svc <name>`), so no supervisor daemon is
needed. Production: systemd units in `ops/`. The host opens no inbound ports. Provisioning,
service units, backups and the restore drill live in `ops/` (`ops/README.md`).

Deploys install a prebuilt release asset instead of building on the host: tagging `vX.Y.Z`
runs `.github/workflows/release.yml`, which publishes `vtv-vX.Y.Z-linux-{x64,arm64}.tar.gz`, and
`mise.toml`'s `[tools."http:vtv"]` tells mise how to fetch it. On the host:
`bash ops/deploy.sh --from-mise <version>` (the build-from-source path stays as a fallback).

Rate limits, four buckets keyed on the client IP (the `CF-Connecting-IP` header when the tunnel
peer is loopback, else the TCP peer):

| Bucket | Paths | Rate | Burst |
|---|---|---|---|
| Api | `/api/*` | 20/s | 40 |
| Auth | `/auth/*` | 5/s | 10 |
| Admin | `/admin/*` | 5/s | 10 |
| Ws | `/ws` (hosted mode only) | 10/s | 30 |

Load test: `node scripts/load_ws.mjs` (60 concurrent sockets locally; the 1,000-socket target
needs several source IPs, tracked as DEBT-022).

Launch requirements: a public repository (the AGPL source offer on `/notices.html` points at it),
the data-terms review (Q2), the trademark review (Q3), and the privacy contact (Q9). CI: fmt,
clippy, tests, audit, Rust-only contract job, both database engines; desktop installers on tags.

## UI/UX

New screens use only the hub theme variables from `hub/theme.js` (graphite, black, light presets), 3 to
6 px radii, 12 px labels and 14 px text, one accent element per view. Contrast fixes measured from the
presets: a derived `--onAccent`, error text in `--text` with a `--bad` icon, `--dim` input borders and
hints. Design record: `.paul/design/taste/hub-theme.md`, `.paul/design/polish-log.md`.

## Design Decisions

1.  One router, two hosts: the desktop app is a thin shell over the hosted site.
2.  Hosted at vtradingview.com for public users; charts public, saved state behind Google sign-in.
3.  Frontend kept; existing response shapes are the contract.
4.  One port: HTTP and websocket on 8770.
5.  Google only, auto login for returning users, account created on first sign-in; desktop signs in
    through the system browser and a `vtradingview://` deep link.
6.  Server-side sessions, not JWT.
7.  Sign-in merge: the server value wins, the local value is kept as a 30-day backup the user can restore.
8.  sqlx Postgres driver over pgsqlite by default; both engines tested in CI.
9.  Key-value `user_state` with versions; a stale save returns 409.
10. MT5 through per-user HTTPS bridge URLs behind an SSRF guard; the bridge stays Python.
11. Logs and restart are admin-only.
12. mise tasks for the dev stack, systemd on the host, plus a new Cloudflare Tunnel.
13. Public deploy (Phase 4) before sync, desktop and MT5.
14. Primary metric: p75 time to first live candle under 2 s; guardrail: zero cross-user data exposure.
15. New screens in the existing hub theme (`hub-native`), text one step above the chart chrome.

## Implementation Phases

| # | Phase | Done when |
|---|---|---|
| 1 | Rust workspace, `hub-server` parity | contract suite passes against Python and Rust |
| 2 | Order flow in Rust | golden fixtures match within tolerance |
| 3 | Database and Google sign-in | sign-in creates one user; CI green on pgsqlite and Postgres 17 |
| 4 | Public deploy | live through the tunnel, no inbound ports, backup restore drill done |
| 5 | Synced state | a layout drawn signed-out appears after sign-in in a second browser |
| 6 | Tauri desktop | installed app signs in via the system browser, same layout |
| 7 | Per-user MT5 bridges | a bridge streams for its owner; every SSRF case refused |

## Review Status

The design doc went through the full EGM review (Stages 5 to 9). The owner concluded each stage with
findings open; none were applied. Reports: `.paul/design/reviews/vtradingview/`.

*   **Blockers open:** CSRF token lost on reload (C1); no Risks section (C2); websocket protocol
    contradicts today's one-socket-per-chart protocol (R1) and its per-IP limit breaks the 8-chart layout
    (R2); tables without columns (R3); new routes without shapes (R4); order-flow cache is parquet under
    `hub/.cache/`, not `DATA_DIR/orderflow/` (R5); `tpo.rs` ports code that does not exist (R6); log ring
    is 12,000 lines, not 2,000 (R7).
*   **Proposed but unapplied fixes:** `.paul/design/reviews/vtradingview/goldfish_review_report.md`.

## Open Questions

1.  **Self-hosting (E1):** is the local, account-free mode dropped on this branch, or kept?
2.  Hosting region and provider (must reach Binance and Bybit).
3.  Deriv, Bybit and Binance data redistribution terms. Blocks the public launch.
4.  The name "vtradingview" and the TradingView trademark. Blocks the public launch.
5.  Windows and macOS code-signing certificates.
6.  Vela's run-time storage key names (inventory, first task of Phase 5).
7.  Global upstream symbol cap value.
8.  Privacy page contact address and final text. Blocks the public launch.

## References

*   Design doc: `.paul/design/specs/vtradingview.md`; plan: `.paul/PLANNING.md`
*   Retired with phase 4: the Python hub (the `hub/*.py` server modules and
    `scripts/capture_orderflow_fixtures.py` were deleted). `bridge/mt5_bridge.py` stays for Phase 7.
*   Reference template: `rust-full-stack-with-auth-template` (owner's machine)
*   mise (tools, tasks, prebuilt binaries): https://mise.jdx.dev; pgsqlite: baysao fork, branch
    `fix/insert-select-returning-timestamptz`

## Licence

**AGPL-3.0-only** (see [LICENSE](LICENSE)), because the bundled Pine Script engine
([PineTS](https://github.com/LuxAlgo/PineTS) and [Vela-pinets](https://github.com/LuxAlgo/Vela-pinets)) is
AGPL-3.0. Running a modified version for other people over a network, as vtradingview.com will, requires
offering them its source. Third-party parts keep their own licences: see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Market data belongs to its providers; check each feed's
terms before any public or commercial use.
