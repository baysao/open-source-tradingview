# 03-03 SUMMARY: header sign-in control, avatar menu, gated log panel, browser check

**Status:** applied, all tasks and checkpoints done (agent-run checkpoint, delegated by the owner under R-6).
**Date:** 2026-11-24
**Files:** `hub/auth.js` (new), `hub/hub.html`, `hub/log_panel.js`,
`crates/hub-core/src/lib.rs` (test-only hook, see Deviation 1),
`.paul/phases/01-rust-parity/01-03-SUMMARY.md` (DEBT-004 discharge).

## What shipped

- `hub/auth.js` (new, 15 KB): one `/auth/me` on load; local mode renders nothing; hosted signed out
  renders the mockup-01 button and loads the GIS script only when signed out; One Tap posts
  `{"credential"}` as JSON with a per-tab `sessionStorage` dismissal flag; signed in renders the avatar
  button and menu (Log panel for admins, Delete account, Sign out); `?signin=failed` renders the notice
  and strips the query with `history.replaceState`; `window.HubAuth = { me, onChange, current }` for
  `log_panel.js`. The credential is never logged or stored.
- Delete account is a native `<dialog>` opened with `showModal()`: no `role="dialog"`, no
  `aria-modal`, no hand-written focus trap or Esc handler (the browser provides them); `autofocus` on
  Cancel; busy state carries a spinner with the label at full contrast; 503 and network failures show
  the mockup-04 error box and swap the button for Try again; `overscroll-behavior: contain` on
  `:modal` and `::backdrop`.
- `hub/hub.html`: `auth.js` deferred after `theme.js`; `#authSlot` at the right end of the bar,
  `position: sticky; right: 0` so it stays reachable when the engine/timeframe buttons make the bar
  scroll; token-only styles for the control, avatar, menu, notice and dialog; `lang="en"` added.
- `hub/log_panel.js`: polls `/auth/me` once. `auth: false` (local mode) keeps today's `/api/logs`
  behaviour; an admin polls `/admin/logs` and `/admin/logs/journal`; anyone else gets no rail button,
  no badge and no interval. Reacts to `HubAuth.onChange` (an admin sign-in adds the entry, a sign-out
  removes it and clears the intervals).

## Modern Web Guidance: retrieved

DEBT-009 discharged. `npx -y modern-web-guidance@latest` is allowlisted again, and three guides were
retrieved before Task 1 and applied:

- **`menu`** (popup menu). Applied: a trigger button with `aria-haspopup="menu"` and `aria-expanded`;
  `ArrowDown` on the trigger opens and focuses the first item; `ArrowUp`/`ArrowDown`/`Home`/`End`
  move within the menu; `Escape` closes and returns focus to the trigger; a pointer click on the open
  trigger closes it; focus moves into the menu only on a keyboard open. The menu is a popover
  (`popover="auto"`) placed by script against the trigger's rect, with the plain `hidden` fallback,
  because the popover CSS here must also work in engines without anchor positioning.
- **`dialog`** (Rule 1: "Use for confirmations, forms, sign-in, destructive actions"). Applied: native
  `<dialog>` + `showModal()` for the delete confirmation, `aria-labelledby` on the heading, `autofocus`
  on the **safe** action (Cancel) per the guide's destructive-confirmation rule, `<form method="dialog">`
  for Cancel, `returnValue` reset on `close`, and the mandatory scroll-containment CSS. The guide's
  "DON'T add `role="dialog"`, `aria-modal`, a focus trap, `inert` or an `Esc` handler" is followed:
  the browser owns all of it, which the check proves (Escape cancels, focus stays inside).
- **`secure-external-subresources`**. Applied: SRI is deliberately **omitted** on the GIS script,
  which is the guide's own exception ("If you must load a mutable third-party script that updates in
  place, omit `integrity`"), with a comment naming the reason at the call site. No integrity hash is
  invented for a URL Google serves mutable.

`validate-origins` was also retrieved to discharge DEBT-004 (see the 01-03 SUMMARY); no change needed
there.

## AC results

| AC | Result | Evidence |
|----|--------|----------|
| AC-1 control matches the mockup in every state | pass | 66/66 browser checks (`check-0303.mjs`); states harness `verify_states` 36 element-states AA in both themes; `axe_audit` 0 violations both themes; menu keyboard parity (click, ArrowDown, ArrowUp, Home, End, Escape, focus return) asserted; `design_lint` red 0 on `hub.html` (its only finding is the file:// console noise of that tool's own loader) |
| AC-2 sign-in, sign-out and delete end to end | pass | button flow `start -> /__test-idp/authorize -> callback -> hub.html` observed in the request log; One Tap POST asserted JSON with `{credential}`; dismissal flag survives a reload in the tab; logout 204; delete dialog 503 and 204 paths, keyboard-only, Escape cancels, focus trapped, focus not obscured (2.4.11) |
| AC-3 log polling gated | pass | signed out and member: 0 `/api/logs`, 0 `/admin/logs`, no rail entry, no badge, and still 0 after 5.5 s; admin: `/admin/logs` polls, `/api/logs` 0, journal 200; a sign-out then admin re-sign-in installs exactly one entry and one badge; local mode: control absent, `/api/logs?brief=1` every 5 s, panel opens and tails |
| AC-4 render and look, accessibility | pass | `verify_states` 36/36 AA (both themes), `axe_audit` 0 both themes, `measure_render` all 30 text elements AA, `verify_responsive` clean at 280..951 px, `lint_intent` OK (destructive never wears primary) |
| AC-5 the checks can fail | pass | four mutations, each applied and verified as applied (L-009), each turning its own check red, then restored green (`mutate-0303.sh`) |

Mutation table (AC-5):

| # | Mutation | Check that went red |
|---|----------|--------------------|
| M1 | log panel starts unconditionally | AC-3 signed out: zero log polls / no rail entry / no badge |
| M2 | admin Log panel entry hidden | AC-1 admin menu contents |
| M3 | One Tap POST form-encoded | AC-2 One Tap POST is JSON |
| M4 | delete runs with no confirm dialog | AC-2 delete: dialog opens, focus on Cancel |

## Gates

- `cargo test --workspace`: 118 passed, 1 ignored.
- `cargo fmt --all --check`: exit 0 after one rustfmt pass on the new `lib.rs` helper.
- `cargo clippy -p hub-core -- -D warnings` and `cargo clippy -p hub-core --features test-idp -- -D
  warnings`: both "No issues found", `cargo clippy -p hub-core -p hub-server --features
  hub-server/test-idp -- -D warnings`: no issues.
- JS syntax: `node --check hub/auth.js`, `node --check hub/log_panel.js`.
- `design_lint.py check hub/hub.html`: red 0 except the tool's own `file://` fetch errors (it loads the
  page from disk; the page needs a server for `/api/*`). No size or contrast finding survived.
- Token/contrast gates on `design-tokens.json`: 123 tokens valid; `validate_contrast` reports one
  pre-existing failure (`semantic.text.link` token missing, both themes) that this plan did not touch
  and cannot fix without inventing a token in `theme.js`; recorded as DEBT-014 rather than fixed here,
  because `theme.js` is the runtime source of truth and a link colour is a design decision.

## Deviations

1. **Test-only hook in `hub-core` (`lib.rs`).** The plan's Facts assumed `hub-server --features test-idp`
   serves `/__test-idp/*`. It did not: `app()` mounts that router only for an **in-memory** Idp, and
   `AppState::start` always builds the production HTTP one, so the mount was unreachable in the binary
   and the browser check had no IdP (probed: `/__test-idp/jwks` 404). Fixed as a 03-02 FIX with the
   smallest change that keeps release builds clean: a `#[cfg(feature = "test-idp")]` `test_idp_deps()`
   that returns `None` unless `VTV_TEST_IDP=1`, and the non-feature twin returning `None` always. With
   the flag on, `start` builds `AuthDeps::in_memory` and the existing mount branch works. The release
   feature tree is unchanged (CI's `cargo tree` assertion still holds: nothing new is pulled in, the
   enum variant and router already existed under the feature).
2. **`#authSlot` is sticky, not merely last.** The bar is `overflow-x: auto` and at 1280 px already
   overflows, so a plain appended control sat 72 px past the right edge. The mockup puts the control at
   the bar's right end, so the slot is pinned there. Found by the check (`gap: -71.89`), fixed, re-run
   green (`gap: 8`). Recorded rather than hidden because it is a layout decision the mockup did not
   spell out.
3. **`design_lint`/`polish.py` cannot measure a served page.** Both load `file://` only; on this page
   that means no `/api/*` and no `/auth/me`, so their console-error rows and any screenshot of the
   control are meaningless. `polish.py shot` was run once, produced only `file://` console errors, and
   its artifacts were deleted; the visual evidence comes instead from the Playwright run's real
   screenshots (`.paul/runs/shots-0303/*.png`) and the kit's render gates. The polish-log entry below
   records what was and was not checked.
4. **The model cannot see images in this session.** The screenshots were captured and are saved, but
   the agent could not visually inspect them; every look-and-feel claim above rests on a measured gate,
   not on a description of a picture. The residual owner row (DEBT-010) already covers the human look;
   this deviation only says the agent did not substitute a claim for a look it could not make.

## Fixed while qualifying

- The account control overflowed the bar (Deviation 2).
- `#logBadge` was `var(--dim)` at 11 px: 4.02:1 on graphite (axe SERIOUS, light and dark). Now
  `var(--text)` at rest and `color-mix(--bad 70%, --text)` when bad; both themes 0 violations.
- `<html>` had no `lang` (axe SERIOUS).
- Two check-script defects of my own, both caught by running it: a locator that also matched the menu's
  decorative avatar, and a `waitForURL` on an intermediate hop that had already resolved.

## Debt

- Opened: **DEBT-014** `semantic.text.link` token missing (pre-existing `validate_contrast` failure),
  owner = owner, phase 04, because a link colour is a design decision (`theme.js` is the token source).
- Opened: **DEBT-015** the browser check needs a live `VTV_TEST_IDP=1` server plus Playwright from the
  kit's `node_modules`, so it cannot run in CI as written; owner = agent, phase 04.
- Discharged: **DEBT-009** (guides retrieved, above), **DEBT-004** (`validate-origins`, 01-03 SUMMARY).
- Still blocking the phase exit: **DEBT-011** (pgsqlite `--host`).

## Next

UNIFY 03-03. It is the last plan of Phase 03, so the phase's `/reflect` (pstack) and RSI run at that
UNIFY, then `transition-phase`.

## Design polish record

Added to `.paul/design/polish-log.md` as screen 08; the summary is: `hub.html` cannot be shot by
`polish.py` (Deviation 3), the control's states were gated on a purpose-built harness at 36
element-states, and the worst three problems found and fixed were the overhanging control, the badge
contrast and the missing `lang`.


## Files

| Path | Change |
|------|--------|
| `hub/auth.js` | new: the control, the menu, One Tap, delete dialog, failed notice |
| `hub/hub.html` | `auth.js` script tag, `#authSlot` markup, token-only styles, `lang="en"` |
| `hub/log_panel.js` | gated start/stop, `base` route, badge contrast |
| `crates/hub-core/src/lib.rs` | `test_idp_deps()` (feature-gated) wired into `AppState::start` |
| `.paul/phases/01-rust-parity/01-03-SUMMARY.md` | DEBT-004 discharge (validate-origins) |
| `.paul/DEBT.toml` | DEBT-004/009 done; DEBT-014/015 opened |
| `.paul/STATE.md`, `.paul/paul.toml` | position: APPLY complete, next UNIFY |
| `.paul/design/polish-log.md` | screen 08 |
| `.gitignore` | `.ds-receipts/` (gate receipts) |
| `.paul/runs/check-0303.mjs`, `check-0303-local.mjs`, `mutate-0303.sh`, `harness-0303.html` | the checks (gitignored; DEBT-015) |


## A mutation that survived, and why that is not a hole

A fifth mutation (M5) put the `openPanelByName` wrapper back inside `start()` instead of installing it
once. It **survived**: the check stayed green then (66/66 after the UNIFY additions). Investigated rather than waved through: the chained wrapper
calls the previous one, and the only thing it does is remove `logwide` and clear `body`, which repeating
is idempotent. So the mutation changes nothing observable, and no honest check can distinguish the two
versions. The install-once form is kept because it does not grow the call chain across sign-in cycles,
and this paragraph exists so the next reader does not mistake a surviving mutation for an untested path.
The four mutations that do change behaviour (M1..M4) all fail their checks.


## Fixes made during UNIFY (phase-scope reflect findings)

Three findings from the reflect panel were real defects in this plan's own output, not style notes:

1. **The test-IdP hook was guarded by an env var alone.** `/__test-idp/*` mints a token for any `sub`
   and sits outside `gate::area()`, so with the `test-idp` feature compiled in, `VTV_TEST_IDP=1` was the
   only wall. `test_idp_deps()` now also requires `cfg!(debug_assertions)`: a release binary refuses
   even if the feature leaked in, and a build needs all three (feature, dev profile, variable) to mount
   the router. The comment in `lib.rs` no longer claims a CI assertion that did not exist.
   Verified: `cargo build --release --features test-idp` then `VTV_TEST_IDP=1` on that binary gives
   `/__test-idp/jwks` 404 while `/auth/me` still answers (hosted mode up, test IdP dark).
2. **That claim was false: `ci.yml` had no `cargo tree` step.** Added one to the audit job. It greps the
   release tree for `rsa` (the feature's only dependency) and exits 1 on a hit. Proven both ways:
   clean tree passes, `--features test-idp` fails.
3. **The sticky account control overlapped the bar's own buttons.** WCAG 2.4.11 applied to the toolbar,
   not just the dialog: `position: sticky` put an opaque panel over `#q`, the timeframe buttons and the
   engine segment at 320, 768 and 1280 px. Restructured: the scrollable content lives in `#barScroll`
   and `#authSlot` sits outside it, so nothing is clipped behind a scroller and nothing is covered. A
   new assertion walks every bar control's visible centre with `document.elementFromPoint` at three
   widths and fails on any hit; it fails on the old sticky-inside-scroller markup and passes now.

Also fixed while re-running: the occlusion assertion first reported clipped elements as covered (a
scroll container hides, it does not overlay); it now intersects each element with every clipping
ancestor before sampling a point. The AC-5 mutation set was re-run after these edits, as it must be
when the checker itself changed: M1..M4 all still fail their own checks and restore green (66/66).


## Reflect and RSI

- reflect: `reflect` (pstack), phase scope over 03-01..03-03, 3 reviewers + synthesizer: 15 raw findings,
  4 accepted, 3 rejected, 5 filed as candidate learnings. Accepted, all with direct evidence:
  probe a runtime Fact before building on it; quote an exit status bare, never through a pipe; an
  incomplete gate set is an unrun gate; editing a checker invalidates its mutation results. They landed as
  `R-7`..`R-10` in `.paul/SPECIAL-FLOWS.md`; the five candidates are `L-019`..`L-023` in `.paul/LEARNINGS.md`
  (candidate status, not promoted: one occasion each); `DEBT-016` and `DEBT-017` carry the two gates the
  findings say should exist.
- instruction edits proposed: `paul-apply` (probe-forever Facts; bare exit status), `design-gate`
  (measured-surface precondition; full receipt set), `ponytail` (re-run mutations after editing a check),
  `a11y-audit` (occlusion sweep), `huashu-design` (file:// precondition). **None applied**: skill edits are
  outside this project's authority and wait for the owner's explicit approval.
- rsi: no candidate. The five new rows are `candidate` and each names its own verification shape; the two
  that would need an experiment (a gate that measures served pages, a single five-gate runner) are the
  debt rows above, so running them now would duplicate work scheduled for Phase 04.
- ledger: added `hub/hub.html` (`design_lint.py ledger add --project vtradingview`).
