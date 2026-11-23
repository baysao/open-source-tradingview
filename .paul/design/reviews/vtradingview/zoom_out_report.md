# Zoom Out Report (Stage 5): vtradingview

*   **Target:** `.paul/design/specs/vtradingview.md` (full file)
*   **Standard:** `~/.agents/egm/resources/document_standards/zoom-out-standard.md`
*   **Reviewer:** `egm-zoom-out` (fresh context, no GoldfishContext; read the linked `.paul/PLANNING.md`,
    `README.md`, `hub/server.py`)
*   **Date:** 2026-11-23
*   **Context sufficiency:** passes.

Elephant check: the facts the findings rest on were confirmed in the repo. `LICENSE` is AGPL-3.0 and
`README.md:100-102` says a modified version run over a network must offer its source.
`hub/server.py:907` returns `ws_port`, `mt5` and `arm` from `/api/symbols`, and `hub/lab.js:95` reads all three.

## Summary

The root problem is addressed. Four Majors say a decision is larger than the problem needs. Three of
them (1, 3, 4) and part of 5 challenge choices the owner made explicitly during SEED (Axum, pgsqlite
by default, Tauri, MT5 option c). Two Majors are defects in the document itself: it says nothing
about the self-hosted product it retires (2), and it contradicts itself on the merge rule (7).

| # | Severity | Finding | Kind |
|---|---|---|---|
| 1 | Major | Full Rust rewrite rests on an unmeasured bottleneck; "async Python, one port, shared FeedHub" not weighed | challenges owner decision |
| 2 | Major | Silent on self-hosters, Local MT5, SSH Remote MT5; `ws_port`, `mt5`, `arm` fields change despite G1 | document defect |
| 3 | Major | Two database engines (pgsqlite fork + Postgres) for one instance | challenges owner decision |
| 4 | Major | Tauri shell that only shows the site; installable PWA not weighed | challenges owner decision |
| 5 | Major | Per-user MT5 bridges (largest security surface) in the first release before demand is shown | challenges owner decision (scope, not mechanism) |
| 6 | Nit | Desktop auth built in Phase 3, three phases before its consumer | sequencing |
| 7 | Major | Q8 and CUJ 3 say the merge rule is open; Technical Plan and Alternative G say decided | document defect |
| 8 | Nit | Custom TTFC beacon and histogram where Cloudflare analytics plus `performance.mark` may do | simplification |
| 9 | Nit | Hand-written rate limiter, sessions, CSRF where `tower_governor`, `tower-sessions` or Cloudflare rules exist | reuse |
| 10 | Nit | Architecture sections carry implementation detail (iss variants, CIDRs, px sizes, line numbers) | altitude |
| 11 | Nit | `setItem` wrapper depends on Vela key names with no plan for keys renamed on upgrade | missing failure mode |
| 12 | Nit | AGPL-3.0 network clause: hosted site must offer source; not in the plan | missing obligation |

## Findings

### Finding 1 (Major): the Rust rewrite rests on an unmeasured bottleneck
*   **Scope:** The Problem; Alternative A; NFR Performance; Phases 1 and 2.
*   **Rationale:** every symptom listed (threaded HTTP server, two ports, pandas in handlers, one
    upstream per viewer) can be fixed inside Python with an async framework on one port and a shared
    subscription per symbol, keeping the numpy/pandas order flow and removing Phase 2 (port, golden
    fixtures, capture script). The only Python alternative weighed is a PyInstaller sidecar. The load
    test that would prove a bottleneck runs in Phase 4, after the rewrite.
*   **Asked:** add the async-Python alternative with a concrete rejection reason, or measure first.

### Finding 2 (Major): the retired self-hosted product is not mentioned
*   **Scope:** Overview; Non-Goals; items 28 and 35; CUJ 7.
*   **Rationale:** item 35 deletes `start.bat` and `start.sh`, item 28 removes the SSH settings, and
    `hub-server` refuses to start without `DATABASE_URL`, `GOOGLE_CLIENT_ID`, `VTV_SECRET_KEY` and
    `PUBLIC_ORIGIN`, so a local run without accounts becomes impossible. G1 promises unchanged shapes,
    yet `ws_port`, `mt5` and `arm` in `/api/symbols` change or disappear.
*   **Asked:** state in Non-Goals that self-hosting, Local MT5 and SSH Remote MT5 are dropped, or keep
    a local mode (`AUTH_ENABLED=false` with no database); list the fields G1 changes.

### Finding 3 (Major): two database engines for one instance
*   **Scope:** G7; Alternative E; Deployment `db` line; Verification "Database engines".
*   **Rationale:** an extra process, an experimental fork on a personal branch, a two-engine CI matrix
    and pgsqlite-shaped migration rules, for a single instance that the Non-Goals say will not scale
    out. Postgres everywhere (or SQLite everywhere) removes a CI dimension and a named risk.
*   **Asked:** keep it if the owner insists, but record the cost against the need.

### Finding 4 (Major): a desktop shell that only shows the website
*   **Scope:** Overview item 3; CUJ 6; Desktop shell; Authentication "Desktop"; Phase 6; Q4.
*   **Rationale:** the desktop flow costs a deep-link protocol (one-time codes, `desktop_key`, a table,
    an exchange endpoint), capability files, three installers and signing certificates. An installable
    PWA gives an app window and taskbar presence with the normal web sign-in and no new attack surface.
*   **Asked:** defer, or add "PWA" as an alternative with a concrete rejection reason.

### Finding 5 (Major): per-user MT5 bridges in the first release
*   **Scope:** G6; CUJ 7; MT5 bridges; Phase 7; Alternative F.
*   **Rationale:** the SSRF guard, vetted-address resolver, encrypted tokens and per-bridge polling
    serve users who run MT5 on a VPS and publish it over HTTPS themselves. The reason Alternative F(a)
    was rejected ("existing users rely on MT5") refers to self-hosters, who are not served by this
    either. Demand for the hosted product is untested.
*   **Asked:** ship without MT5; add Phase 7 once demand is shown.

### Finding 6 (Nit): desktop auth three phases early
*   **Scope:** Authentication "Desktop" ("specified and tested in Phase 3"); item 19.
*   **Rationale:** a speculative interface with no consumer until Phase 6; unnecessary if Finding 4
    stands.

### Finding 7 (Major): the document contradicts itself on the merge rule
*   **Scope:** Q8; CUJ 3; "Sign-in merge (decided ...)"; Alternative G.
*   **Rationale:** Q8 and CUJ 3 say the rule is open; the Technical Plan and Alternative G say it is
    decided. A reader cannot tell which holds.
*   **Asked:** close Q8 and update CUJ 3.

### Finding 8 (Nit): custom metrics for one number
*   **Scope:** G2; CUJ 9; Admin and metrics; items 26 and 32.
*   **Rationale:** Cloudflare Web Analytics gives page-load percentiles; "first live candle" may only
    need a `performance.mark` read by an existing tool. Justify first-party code or simplify.

### Finding 9 (Nit): hand-written middleware
*   **Scope:** request handling; items 9 and 21.
*   **Rationale:** `tower_governor`, `tower-sessions` with a sqlx store, a maintained CSRF crate, or
    Cloudflare rate-limiting rules exist. Prefer them unless a requirement rules them out.

### Finding 10 (Nit): implementation detail at architecture altitude
*   **Scope:** Authentication; MT5 bridges steps 1 to 4; New screens; item 25.
*   **Rationale:** iss variants, JWKS refetch interval, cookie attributes, CIDR lists, pixel sizes,
    contrast measurements and source line numbers belong in tests, a security appendix, or
    `taste/hub-theme.md`.

### Finding 11 (Nit): no plan for Vela key changes
*   **Scope:** Saved state and sync, client side; item 40.
*   **Rationale:** sync depends on Vela key names chosen at run time (Q5). A Vela upgrade that renames
    a key silently stops syncing it. State the limit and how an unknown key gets noticed.

### Finding 12 (Nit): AGPL-3.0 source offer
*   **Scope:** Q2, Q3; NFR Rollout; item 33.
*   **Rationale:** public hosting of a modified version triggers the network clause. Add a source link
    or offer beside the licence page and make it a launch condition.

## What holds up

One router and one port; server-side sessions over JWT; the shared FeedHub with explicit limits;
the contract suite against both servers; the closed origin behind the tunnel; tight Non-Goals.
