Status: resolved (owner replied "continue" on 2026-11-23: no fixes applied; E1 unanswered; Blockers C1 and C2 remain open in the spec)

# Goldfish Annotated Review Report

**Target:** `.paul/design/specs/vtradingview.md` (full file, lines 1 to 914)

Sources: [critic_report.md](critic_report.md) (Stage 7, ids C1 to C25 and the grouped Nits),
[zoom_out_report.md](zoom_out_report.md) (Stage 5, ids Z1 to Z12),
[comprehension_report.md](comprehension_report.md) (Stage 6, ids K1 to K16). Findings repeated across
stages are resolved once under the first id.

Counts: 1 ESCALATE, 3 CONTEST, 41 PROPOSE FIX, 12 DECLINE.

## Escalated (needs the owner's knowledge)

### [E1] [`egm-zoom-out` Z2, `egm-comprehension` K3] - Severity: High
*   **Affected Scope:** Overview; Non-Goals; Goals G1; items 28 and 35 (`start.bat`, `start.sh`, SSH
    settings); `hub-server` start-up rule (refuses to start without `DATABASE_URL`, `GOOGLE_CLIENT_ID`,
    `VTV_SECRET_KEY`, `PUBLIC_ORIGIN`).
*   **Discrepancy Description:** the spec retires the self-hosted product (local run without accounts,
    Local MT5 via `LOCAL_MT5=1`, Remote MT5 over SSH, the restart button) without saying so, and G1's
    "every route keeps its shape" is false for `/api/symbols` (`ws_port`, `mt5`, `arm`; read by
    `hub/lab.js:95`).
*   **Rationale:** a reader cannot judge what is lost; README still sells "free, self-hosted".
*   ***AI Suggestion:*** ESCALATE
*   ***Justification:*** whether self-hosting survives was never asked during SEED. The hosted plan was
    agreed, but not whether the open-source local mode is dropped. Two defensible answers:
    (a) **drop it on this branch**: Non-Goals gain "self-hosting, Local MT5 and SSH Remote MT5 are dropped;
    the last Python release stays tagged for self-hosters"; or
    (b) **keep a local mode**: `hub-server` starts with `AUTH_ENABLED=false` and no database, binds
    `127.0.0.1`, serves charts and feeds only, and `start.bat`/`start.sh` run the Rust binary instead of
    being deleted. Local MT5 still needs the Python bridge (option c URL pointing at `127.0.0.1` would be
    refused by the SSRF guard, so local mode would need a guard exemption for loopback when auth is off).
    Either way G1 gets an explicit list of changed fields: `ws_port` removed, `mt5` and `arm` replaced by
    per-bridge status.
*   ***Proposed Edit:*** applied after the owner picks (a) or (b).

## Blockers

### [C1] [`egm-design`] - Severity: High
*   **Affected Scope:** Lines 293-300 (Sign-in effect, Requests), 317-322 (sync), 718 (item 24).
*   **Discrepancy Description:** CSRF token returned only at sign-in, kept in memory, stored hashed; lost
    on reload or in a second tab; every write fails until a new sign-in.
*   **Rationale:** breaks G4 and CUJ 4 on the first reload.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** valid bug, confirmed at lines 295 and 718.
*   ***Proposed Edit:***
    *   Location: Authentication, "Sign-in effect" and "Requests"; item 21 `csrf.rs`; item 24; item 23.
    *   Change: the CSRF token is derived, not stored: `HMAC-SHA256(VTV_SECRET_KEY, session_id)`,
        base64url. `/auth/me` returns it with the user on every page load, so every tab and every reload
        gets the same valid token; the server recomputes it to check `X-CSRF-Token`. Revoking the session
        revokes the token. Remove "and of the CSRF token" from the stored hashes and the
        `csrf_token_hash` column. Tests: reload, second tab, and revoked session cases in
        `crates/hub-core/tests/auth.rs` and `hub/tests/sync_harness.js`.

### [C2] [`egm-structural`] - Severity: High
*   **Affected Scope:** between Alternatives Considered and Non-Functional Requirements.
*   **Discrepancy Description:** no Risks & Mitigations section; the standard requires one.
*   ***AI Suggestion:*** PROPOSE FIX
*   ***Justification:*** valid; the only risk table is in PLANNING.md, outside the spec.
*   ***Proposed Edit:*** new `## Risks & Mitigations` table (risk, likelihood, impact, mitigation,
    trigger): pgsqlite defects (trigger from C24); upstream bans or region blocks; data redistribution
    terms (Q2); trademark (Q3); AGPL source offer (Z12); Google blocks webview sign-in; order-flow drift;
    SSRF; remote page gains Tauri IPC; global feed cap exhaustion (C14); shared-NAT socket limits (C15);
    metric poisoning (C16); Vela key renames (Z11); single host and up to 24 h backup loss (D24);
    Cloudflare Tunnel outage.

## Major: PROPOSE FIX

| Id | Proposed edit (location: change) |
|---|---|
| C3 (=Z7, K1) | Q8: replace with "Resolved: server wins, local value kept as a 30-day backup (see Sign-in merge)". CUJ 3 Then: state the decided rule and the backup notice. |
| C4 | New screens: each screen row links its mockup in `../opendesign/vtradingview/` (`01-signin-avatar-menu.html` to `07-privacy.html`, plus `DESIGN.md`); one sentence: the mockups are the normative layout, element order and state rendering; the spec text wins only where it states a rule. |
| C5 | Full variable list: the 14 `theme.js` keys plus `--font`, `--mono`, derived `--onAccent`, and size tokens `--fs-xs` 12 px, `--fs-sm` 14 px, `--fs-md` 14 px. New plan item: `hub/account.css` (C), loaded by `hub.html`, holds the size tokens, focus ring, motion tokens and the new screens' CSS (ported from the mockups' `shared.css`). `theme.js` stays the only colour source; `design-tokens.json` is a generated reference for tooling, not imported by the product. |
| C6 | Add a control states table: every button and menu item has default, hover (`--raised` background), focus (2 px `--accent` outline, 2 px offset: light 3.39, graphite 8.39, both above 3:1 for non-text), pressed (`--line` background), disabled (opacity 0.5, no pointer events), busy (opaque, spinner, `aria-busy`). Primary hover: `color-mix(in srgb, var(--accent) 88%, var(--text))`. Accessible names: avatar button "Account menu for <name>", restart "Restart <feed> feed", Restore "Restore <key> from <date>", bridge delete "Delete bridge <label>". |
| C7 | Add states to the screens table: sign-in failed ("Sign-in did not finish. Try again." with the reason), avatar image fails or absent (initial letter on `--raised`), session expired on a write (401: "You were signed out. Sign in again to keep saving."), value over 1 MB ("Too large to sync: <key>, kept in this browser"), key limit reached during merge, MT5 list loading, MT5 bridge previously connected now failing (`last_error` with time), admin loading, empty log, admin socket disconnected, feed data stale, TTFC with fewer than 50 samples ("not enough samples"). |
| C8 | Admin panel Dashboard Rule: question "Is any feed failing viewers right now?"; numbers per feed: lag (seconds, 1 decimal, baseline under 2 s), seconds since last message, reconnects in the last hour, open upstream subscriptions vs cap; TTFC p75 (seconds, 2 decimals, window and sample count shown). Problem rules: lag over 5 s, or no message for 30 s, or 3 reconnects in 10 min, marks the row as a problem with Restart beside it; TTFC p75 over 2 s marks the TTFC figure. Known-totals fixture: a fixed log and feed-state JSON with expected row states, used by a Playwright test in Phase 3. |
| C9 | Replace the UI gates list: per screen, `verify_states.mjs`, `axe_audit.mjs`, `verify_responsive.mjs`, `verify_keyboard.mjs`, `verify_target_size.mjs`, `verify_reduced_motion.mjs`, `verify_focustrap.mjs` (delete dialog) from `~/.agents/ux-ui-agent-skills/scripts/`, and `lint.mjs` from `~/.agents/skills/design-taste-frontend/scripts/`, with `DS_REQUIRE_BROWSER=1`. `accuracy_report.mjs` is removed (kit self-test). Presets and fonts are forced by query string on a harness page (`?preset=graphite\|black\|light&font=system\|nunito\|manrope\|quicksand\|inter`); every gate runs on all 3 presets and on the system and Quicksand fonts (narrowest and widest). Built screens are gated on `hub-server --release` serving `hub.html` with a fixture feed (`FEED_FIXTURE=1`). FAIL blocks the phase; WARN is logged in the phase summary. |
| C10 | Add the missing measured rows: light `dim` on `raised` 4.46 (fix: no `dim` text on `raised` in light; menus use `panel`); light `ok` on panel 4.45 (fix: status text in `--text`, `--ok` only on the icon); light `accent` on panel 3.39 (accent never used for text). Destructive style: `--bad` 1 px border, `--bad` icon, `--bad` 12% tint background, label in `--text` (ratios 11.16 graphite, 14.50 black, 12.55 light, measured by the D4 lint pass). User overrides beyond `--onAccent` are not contrast-guaranteed: Settings shows a warning when a chosen pair falls under 4.5. |
| C11 | Google UI: use the GIS-rendered button (Google branding rules; it is not themed and does not count as the view's accent element). One Tap with FedCM: prompt position is browser-controlled, accepted. `auto_select: true` stays, because the owner asked for auto login (PLANNING.md "auto login / auto create user"): CUJ 2 rewritten: a returning user who signed in before is signed in with no click; a first-time user clicks once. The sign-in merge then runs; a notice appears only when backups were created. |
| C12 | Phone layouts per screen: header shows the avatar (or the GIS icon button) only, menu opens as a full-width sheet; MT5 form stacks fields at full width; admin panel stacks health above the log, log scrolls horizontally inside its box. Known defect carried: bridge URLs wrap only at `/`, `.` and `:` (no mid-number breaks). |
| C13 | Add a copy deck section with the exact strings (conflict, backups created, delete confirm naming account, sessions, saved state, MT5 bridges, and "This cannot be undone", each MT5 error: "Address refused: <host> points to a private or local network", "Certificate error for <host>", "Wrong token", "No answer within 10 s", MT5 empty state, desktop failed "Sign-in did not finish in the browser. Try again, or check that your browser is not blocking pop-ups"). Privacy page outline: data held, retention (sessions 30 days sliding, state backups 30 days, server backups 14 days), processors (Google sign-in, Cloudflare), deletion, contact. New Q9: privacy contact address and final text, blocks the public launch. |
| C14 | FeedHub cap counts distinct upstream subscriptions as the provider sees them (source + symbol + the provider's interval key). The default chart's subscriptions are opened at start-up and never count against the cap. Anonymous budget: at most 24 distinct keys per IP across its sockets; signed-in: 64. Each provider connection sends subscribe messages through a rate-limited queue at the provider's published limit. |
| C15 | One websocket per tab (the engine page's `lab.js`; layouts of up to 8 charts share it). Per-IP limit raised to 20 sockets, so shared NAT and multi-tab users fit; abuse is bounded by the per-IP key budget (C14) instead. |
| C16 (=K12, Z8) | One name and definition: "time to first live candle (TTFC): navigation start to the first live candle update rendered on the default chart". The histogram is persisted as per-day bucket counts in a `metrics_ttfc_daily` table (flushed every minute), so deploys do not erase the week. Poisoning bound: values outside 0 to 60,000 ms dropped, one sample per IP per minute; Cloudflare Web Analytics page-load percentiles are the cross-check. |
| C17 | Motion decision: menus, notices and the desktop overlay fade and rise 4 px over 120 ms (`--dur-fast`, ease-out); under `prefers-reduced-motion: reduce` they appear with no transition; the chart has no new motion. Tokens live in `hub/account.css`. |
| C18 | Every Reason for Rejection leads with the technical or cost reason; the owner's choice follows as provenance. B: "two server deployments to keep in step (they drifted in the template); a local backend cannot hold synced accounts without the hosted database". I: "a new look would read as foreign inside an existing product's chrome". |
| C19 | Promote Additional Resources, Future Improvements and Verification Plan to H2. |
| C20 | Replace "measured in this session" with "computed with the WCAG 2.x relative-luminance formula from the preset hex values in `hub/theme.js`; record in [taste/hub-theme.md](../taste/hub-theme.md)". |
| C21 (=K5) | Define D4 at first use: "the design track's mockup step (D4), whose screens were checked with huashu-design's `design_lint.py`". |
| C22 (=K8) | `desktop` depends on `hub-types` for the public origin and deep-link scheme constants. |
| C23 (=K15) | Bars cache: newest window (the request with no `end`) 5 s TTL; older windows 10 min. |
| C24 | Switch trigger: production moves to Postgres when a pgsqlite-only CI failure is not fixed within 5 working days, or on any production data error or crash traced to pgsqlite. |
| C25 | Load mix: 1,000 connections, average 2 subscriptions each, 50 distinct symbols across Deriv, Bybit and Binance, 70% M1 and 30% H1; pass when held for 30 min with no dropped connection and upstream subscriptions at 50. |
| K4 | Desktop handoff without exposing `desktop_key` to the page: the Rust deep-link handler receives `code`, then navigates the window to `https://vtradingview.com/auth/desktop/complete?code=<code>&dk=<desktop_key>`; the server checks code and key, sets the session cookie in the webview and redirects to `hub.html`. The page never holds the key; the remote origin's only permission is `start_sign_in`. `/auth/desktop/exchange` (POST) becomes `/auth/desktop/complete` (GET, single use, 60 s). "Try again" generates a new `desktop_key`; a deep link arriving before the window loads is held by Rust and applied on load (D23). |
| Z1 | New Alternative J "async Python on one port with a shared FeedHub": pros (keeps numpy/pandas, removes Phase 2); cons (Python runtime and three scientific packages stay on the host; two languages if the desktop shell is Rust); reason: the rewrite is not justified by a measured bottleneck (none was measured); it is chosen for a single static binary to deploy and a single language with the Tauri shell, and by the owner. Phase 1 adds a baseline: run `load_ws.mjs` against the Python server before porting, recorded for comparison. |
| Z3 | Alternative E: add the cost line (extra process, a fork on a personal branch, a two-engine CI matrix) against the benefit (SQLite file copy for dev and branch databases), and the C24 trigger. |
| Z4 | New Alternative K "installable PWA": pros (no installer, normal web sign-in, no new attack surface); cons (Firefox desktop has no PWA install, no tray, no OS deep links, store and IT policies that require an installer); reason: the owner asked for a desktop app; a PWA manifest is still added in Phase 4 as a cheap extra, so browser users get an app window too. |
| Z12 | AGPL-3.0: the hosted site links its exact source (repository URL plus commit) from the footer and `notices.html`; item 33 gains it; added to Risks and as launch condition next to Q2 and Q3. |
| Z9 | Rate limiting uses `tower_governor`; sessions stay first-party because of the hashed-id schema and the desktop handoff. |
| Z11 | `sync.js` reports keys written by Vela that match no allow-list pattern (debug log plus a count on the admin panel); `sync_harness.js` fails when an unknown Vela key appears, so a Vela upgrade cannot silently stop syncing. |
| K11 | Q5: the inventory is the first task of Phase 5 and gates the rest of the phase. |
| K13 | CUJ 2: "does not reappear in this tab". |
| K14 | Overview gains one line: TradeX is the product, "the hub" is the `hub/` frontend, vtradingview is the hosted site and this project; engines are Vela, Lightweight Charts (lw) and KLineChart (kline). |
| K16 | Backups live on the server only; the Restore action fetches `backup:*` keys; `sync.js` never writes them to `localStorage`. |
| K2 | gaffa defined at first use: "gaffa (a Procfile process supervisor, https://github.com/oryon-dominik/interstellar-gaffa)"; pricehub: "a sibling project of the owner whose gaffa and pgsqlite setup is copied". |

## Contested

### [Z5] [`egm-zoom-out`] - Severity: Medium
*   **Affected Scope:** G6, CUJ 7, MT5 bridges, Phase 7.
*   ***AI Suggestion:*** CONTEST
*   ***Justification:*** the finding says MT5 bridges ship "in the first release". The spec's own phase
    order puts them last: Phase 4 is the public launch, Phase 7 is per-user MT5 (Detailed Implementation
    Plan, "Phase 7"), and NFR Rollout already ships later phases onto the live site. The finding's real
    point (demand untested) is covered by adding to Phase 7: "start only after Phase 4 has live users".

### [C11-part, design D11 "one click"] - Severity: Medium
*   ***AI Suggestion:*** CONTEST (the `auto_select` part only; the rest is fixed above)
*   ***Justification:*** auto sign-in without a click is the owner's stated requirement, readable in the
    linked PLANNING.md ("Google sign-in with auto login / auto create user"). The defect is CUJ 2's
    wording, fixed above, not the setting.

### [Z10] [`egm-zoom-out`] - Severity: Low
*   ***AI Suggestion:*** CONTEST
*   ***Justification:*** the security values (issuer strings, cookie attributes, CIDR ranges, limits) are
    pinned in the spec on purpose: the Detailed Implementation Plan must be implementable without
    impactful decisions (egm-draft Step 5 requirement), and the tests cite them. Moving them out would
    leave the tests without a source.

## Declined (Nits, recorded, not fixed)

Z6 (desktop auth specified in Phase 3: kept, its schema is cheap and K4 changes it anyway); K6 (seed and
draw: covered by the header link to direction-approved.md); K9 (reference template order); S5
(Alternatives F and G shape); S6 (header names differ from the standard but map clearly); T11, T13, T14,
T20, T21, T25, T28 (wording preferences). D18 (Idea, Argument, Imagery rules): a one-line statement is
added with C4 rather than a separate section.

## Nits fixed in the same batch (cheap)

S8 (split C/M/D markers in items 35 and 52); S9, T30 (re-wrap lines 368 and 762, drop the double
blank line); S11, T29 (`crates/hub-core/tests/auth.rs` full path); K7 (one entry-point signature
`app(cfg: Config, state: AppState) -> Router`); T12 ("per viewer"); T15 (CUJ 1 "within the G2 target");
T17 ("the workspace defines no other `Router`"); T18 (no `CF-Connecting-IP`: the request is keyed as
`127.0.0.1` in the limiter and gets no other trust); T19 (branch named `arch/axum-tauri`); T22 ("one
`--accent` element per view"); T23 (ratios tied to presets, 1.25 graphite, 1.30 black, 1.32 light);
T24 (rule source: `CLAUDE.md` house rules, `scripts/lint_native_select.py`); T26 (Vela key count marked
unknown, Q5); T27 ("one shared version per dependency, recorded in `Cargo.toml`"); D19 (polish-log
Screen 05 status line updated to the orchestrator's final red 0); D20 (Overview says what changes in
the chart UI); D21 (sign-in reopens the websocket); D22 (merge compares canonical JSON: keys sorted, no
whitespace); D24 (single host, up to 24 h backup loss, stated as accepted in Risks); S10 (`~/` paths
marked "on the author's machine").
