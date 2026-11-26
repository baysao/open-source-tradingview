# Polish log (index)

## D4 vtradingview  (screens: .paul/design/opendesign/vtradingview/, direction hub-native)

Status: done, red 0 on all 7 screens (design_lint.py check, final run by orchestrator 2026-11-23)

### Orchestrator round (after the 7 parallel lint passes)
- Worst 1: each lint agent had lifted text sizes with its own page-local override (seven different scales, one with !important), breaking one theme one source.
- Worst 2: admin log panel still red 2 (log lines and labels below the size floor); the agent had accepted them instead of fixing.
- Worst 3: a disabled "Restored" button on 02 at 4.07 contrast showing a finished state.
- Changed: type floor moved into shared.css (--fs-xs 12, --fs-sm 14, --fs-md 14; prose classes on --fs-md; uppercase tracked labels to sentence case; busy buttons opaque, disabled stays opacity .5); page-local size overrides removed from 01 to 06; admin log and feed meta on --fs-sm; Restored button removed (the row text says it); split-emphasis log heading made plain.
- Lint: red 0 on 01 to 07; yellow kept: mono labels on 02 (6), 03 (4), 05 (47), reason: timestamps, storage keys and log lines are data identifiers, hub-native.
- Looked at after the change: 03 mobile frame 2, 05 desktop frame 1. Left: on mobile a long bridge URL wraps inside the port number.
- Spec impact: new screens use 12 px labels and 14 px text, one step above the hub's 11 to 13 px chart chrome. Recorded for D5.

### Screen 01
## D4 vtradingview: 01-signin-avatar-menu

Round 1
- Worst: copy under 14px (lint red x3); "Signing in" label 4.07:1 at half opacity; frame titles with one differently styled word and mono micro labels.
- Changed: page-local overrides, copy 14px, labels 12px+, busy button full opacity, frame title single style, tabular sans replaces mono time labels.
- Lint: red 1, yellow 1 (admin frame title 13px)
Round 2
- Worst: frame title 13px; mobile and desktop frames otherwise clean.
- Changed: frame titles 14px.
- Lint: red 0, yellow 0

Status: done, red 0, yellow 0

### Screen 02
## D4 vtradingview 02-sync-notices
Round 1: worst: body text under 14px (18 spots), labels at 11px, disabled Restored button contrast 4.07.
Changed: page-local :root --fs-xs 12, --fs-sm/md 14; disabled .btn opacity 1 with dim colour; heading <b> colour reset.
Lint: red 3, yellow 2.
Round 2: worst: backup description rows still 12px; mono tags; mobile frames stack cleanly.
Changed: .key span to fs-sm. Kept mono tags (timestamps, storage key names): data identifiers, hub-native direction fixed by owner.
Lint: red 0, yellow 1.
Status: done, red 0, yellow 1 (mono labels kept, reason above)

### Screen 03
## D4 vtradingview (03-mt5-bridges)
Round 1: worst: body and label text under floors (11-13px), Testing button 2.79:1 contrast, uppercase tracked labels. Lint red 3, yellow 2.
Round 2: page-local type lift (xs 12, sm 13, md 14 for prose), busy button opaque, sentence-case section labels, titles de-split. Lint red 1, yellow 1.
Round 3: mono URL and code lifted to 14px. Lint red 0, yellow 1 (mono small labels: kept, hub-native direction fixed by owner).
Status: done, red 0, yellow 1

### Screen 04
Round 1 (04-delete-account):
- Worst: body copy 12-13px under the 14px floor; Busy Cancel/Deleting faded to 3.9-4.1:1; "cannot be undone" in dim grey.
- Changed: page-local 14px body, busy button opacity 1 (spinner carries state), warning line in full text color on its own line; removed single-word bold in frame title.
- Danger label contrast measured: graphite 11.16, black 14.5, light 12.55.
- red 0, yellow 0
Status: done, red 0, yellow 0.

### Screen 05
## D4 vtradingview / 05-admin-log-panel
Round 1: worst three: Bybit row said Connected while its button said Restarting; feed meta line wrapped to two lines beside the button; busy Restarting button failed contrast (4.07).
Changed: Bybit status now Restarting with spinner; meta spans full row; busy button keeps full opacity and text colour (still non-interactive).
Lint: red 2, yellow 2 (red left = hub-native 11/12/13px sizes, kept: hub-native direction fixed by owner; yellow = hub uppercase labels and mono labels, kept for the same reason).
Dashboard-design three worst: honesty (Bybit status contradicted its restart), salience (p75 1.42 s leads, ok), words audit (Error row says what is happening and the attempt).
Status: done, red 2 accepted as hub-native, yellow 2 kept

### Screen 06
## D4 vtradingview
Status: 06-desktop-signin-waiting linted, red 0, yellow 0.
Round 1: worst: (1) body copy 13px under floor, (2) 11px TradeX labels, (3) busy "Signing in" button 4.07 contrast.
Changed: page-local overrides to 14px copy and 12px labels, busy button opacity 1; frame headings plain text.
Lint: red 0, yellow 0. Kept: hub-native direction fixed by owner. Note: Playwright chromium installed to run shots.

### Screen 07
### 07-privacy
Round 1: worst: draft note 13px under 14px floor; meta line small and dim; text-only screens (kept: long-form legal page, no graphics by design).
Changed: draft note to fs-lg, meta to fs-md. Lint: red 0, yellow 0.
Status: done, red 0, yellow 0; contact address still marked missing (draft note and Contact section).

### Screen 08
## 03-03 / hub.html header control, avatar menu, log gating
Round 1: `polish.py shot hub/hub.html` cannot measure a served page (it loads `file://`, so `/api/*` and
`/auth/me` fail and its only output is console noise). Artifact deleted. States measured instead on a
purpose-built harness reusing hub.html's own auth CSS: 36 element-states, AA in dark and light.
Worst three found and fixed by the gates and the browser check:
1. The account control sat 72 px past the bar's right edge at 1280 px (the bar scrolls); now sticky to the right edge, gap 8 px.
2. `#logBadge` at `var(--dim)` 11 px measured 4.02:1 on graphite (axe SERIOUS, both themes); now `var(--text)` at rest, bad state `color-mix(--bad 70%, --text)`.
3. `<html>` had no `lang` (axe SERIOUS); added.
Lint: axe 0 violations both themes; verify_states 36/36 AA both themes; measure_render 30/30 text AA;
verify_responsive clean 280..951 px; lint_intent OK. design_lint on hub.html: red 0 apart from its own file:// fetch errors.
Status: done, red 0. Not visually inspected by the agent (no image input this session); owner look is DEBT-010.

## 2026-11-25 hub.html desktop sign-in overlay (06-03)

Satisfied via the gate-runner note (surface in the receipt set): gate_all 8/8 on hub.html,index.html
(auth-off base) plus a served browser pass with screenshots instead of polish.py shot. The pass found
two real defects the state checks missed: (1) `.desk-card .steps` kept `display:grid` over the UA
`[hidden]` rule and rendered inside the waiting card - fixed with
`.desk-card .steps[hidden] { display:none }`; (2) the failed card's heading was an h3 under the page's
only h1 (axe heading-order) - demoted to h2. `deskHide`'s flag param also shadowed `render()` (Cancel
left the busy button) - renamed `rerender`. Lint: axe 0 violations waiting + failed (driver case 12);
verify_states/verify_responsive green on the auth-off base. Status: done, red 0.

## 2026-11-26 settings.html MT5 bridges (07-04)

Satisfied via the gate-runner note (surface in the receipt set): gate_all 8/8 gates × 5 pages on the
auth-off base (`--base http://127.0.0.1:18774`, settings.html in the page list; `--check-receipts`
exit 0) plus a served browser pass with screenshots (`.paul/runs/shots-0704/`) and the driver's
DOM-level assertions (`.paul/runs/check-0704.mjs`, 14/14, mutation red-then-green). The kit's
`design_lint.py check` is file://-only (`collect` resolves `as_uri()`), so its served half is
gate_all's design_lint receipt — green (red 0). The file:// run reports its own fetch noise (L-020).
Worst three found and fixed by the gates + browser pass:
1. Wrapped inline link in the signed-out panel: the union rect spans both lines and the point
   between them hit-tests to the parent `p` — verify_states read `covered: 1`. Fixed with
   `a.link { white-space: nowrap }`.
2. `lint_intent` wanted a page footer (hasFooter false); added the privacy.html footer pattern
   ("Open the hub | Licence and notices").
3. The page's boot `/api/me/mt5` fetch 404'd on the auth-off base — one console error (design_lint
   red) and, on a real hosted server, a signed-out console error on every visit. Fixed: `/auth/me`
   first; signed out or local mode never touches `/api/me/mt5`. Also fixed en route: 4 hardcoded px
   → rem (lint_hardcodes), and an unconsumed error body that kept the boot request alive forever
   (networkidle never settled; `r.text()` on every path).
Status: done, red 0. Not visually inspected by the agent (no image input this session); the owner
look joins 07-04's checkpoint.

## 09-01 settings.html: the pairing dialog (2026-11-26)

Round 1 (agent, Playwright against a served auth-off hub)
1. The gates never see the dialog: `gate_all.py` measures the page at load, where a `<dialog>` is
   closed, so an 8/8 receipt proves nothing about the panel the user actually reads. Measured the
   open state by hand instead (`showModal()` + a fake code): `dialogOpen: true`,
   `codeVisible: true`, both visible focusables (`Copy code`, `Close`) `covered: false`, Esc closes,
   the code clears from the DOM on close.
2. The mint action had to sit beside `Add` in a `<h3>` that was `display:flex` with the ghost button
   pushed right by `margin-left:auto`; a second button would have split them across the row. Wrapped
   both in `.h3-acts` and dropped the per-button auto margin, so the pair of actions travels together
   and wraps as one unit below 600px.
3. Empty-state copy still told the user to run `bridge/mt5_bridge.py`, deleted in phase 08 (7317378).
   Rewritten to name the manager app or a hand-added bridge; a stale instruction is a defect a lint
   gate cannot catch.
- Lint: `design_lint` pass, red 0. `lint_hardcodes` pass (the dialog uses only theme variables: one
  `--fs-2xl` mono readout, no hex, no px sizes beyond the token scale).
- Looked at after the change: settings.html at 1280 with the dialog open (screenshot
  `.paul/runs/tmp/dialog-open.png`). Left: nothing worth fixing; the code block is the page's one
  large readout by design.
Status: done, red 0. The open-state judgement is the agent's measurement, not an owner look; it
joins 09-03's phase checkpoint if the owner wants one.

## 09-02 mt5-manager ui/index.html: the manager panel (2026-11-26)

Round 1 (agent, gate runner .paul/runs/09-02-gates.py against a served ui/, both themes via
?theme=light — gate_all.py cannot serve a page outside hub/, so the runner imports gate_all and
reuses its five harnesses verbatim)
1. axe color-contrast, 2-3 nodes both themes: danger buttons used --bad as TEXT (3.66 dark /
   4.15 light on a raised panel) and footer used --faint (2.61 / 2.27). The red moved to a 2px
   BORDER (border contrast is not text contrast) and the footer to --dim at 14px.
2. One light-theme node left: the hub's own light preset pairs accentInk #ffffff with accent
   #b8761a — 3.72:1 at 14px, below AA. Deviation, documented in the CSS: the primary button's ink
   is --text in light mode (4.62:1), which is the graphite preset's own dark-ink-on-amber pattern.
   The token PAIR was the defect, not a token value.
3. design_lint: 4 body texts at 13px below the 14px floor (.note, .hint, .result .next, .secret
   code). All to 14px.
4. axe region: h1 + lede sat outside a landmark. Wrapped in <header>.
5. verify_states: the MCP-token input measured unnamed (the gate reads aria-label/text/value, not
   <label for>). Explicit aria-label on every input; the label elements stay for the caption.
6. Every load logged one favicon 404 (browsers request it unprompted). Killed with a data-URI icon.
- Gates after: 10/10 (verify_states, axe_audit, measure_render, lint_intent, design_lint × dark +
  light), verify_responsive both themes (0 overflow, 0 covered at 320/768/1280), dialogs measured
  OPEN by hand (09-01's round-1 lesson): both showModal, focus lands inside, Esc closes, 0 console
  errors.
- Looked at after the change: measure_render.dark.png + measure_render.light.png (Read as images).
  Left: nothing worth fixing; empty states name cause and next action ("Preview mode: this page is
  open outside BridgeManager…").
Status: done, red 0. The native-window look (title bar, no tray) is not judgeable from a served
page; it joins 09-03's real-host checkpoint.

## 09-03: panel reordered into numbered steps, MCP address field added (2026-11-27)

- **Why:** the owner reported the five zones "confused". They were ordered Pairing → Status → Control
  → Install → Unregister, which puts step 2 before step 1: you cannot pair a bridge that is not
  installed, and Unregister sat above Install. A panel read top to bottom told the operator to do the
  third thing first.
- **Change:** `<main>` is now a real `<ol class="steps">` of five numbered cards in operating order —
  1 Install or repair → 2 Pair with your account → 3 Check it is running → 4 Start or stop the
  services → 5 Unregister. Headings rewritten as actions ("Check it is running", not "Status"); the
  lede says "five steps, top to bottom"; step 5 carries a dashed border and its note says it is
  terminal, not a way to re-pair.
- **The number is not a heading decoration.** First attempt put `<span class="step-no">` inside each
  `<h2>`; design_lint flagged it in dark (`标题里只给一个词换色 5 处`) as one-coloured-word heading
  decoration, and the finding differed between themes, which is how a colour-reading lint betrays
  itself. Fixed by moving the badge into a `.step-head` flex row beside a clean `<h2>` — lint clean
  in both themes. The `<ol>` still conveys position to assistive tech, so the badge is `aria-hidden`.
- **Contrast:** the badge is accent-background. In the light theme `--accent-ink` (#ffffff) on
  #b8761a measures **3.72:1** — the same AA trap already documented for `button.primary`. Added
  `:root[data-theme="light"] .step-no { color: var(--text) }`, which measures **4.62:1**. Dark stays
  8.34:1.
- **New field:** step 1 gained **MT5 MCP address** (`#mcp-url`, default
  `http://127.0.0.1:22346/mcp`), because `bridge.env` previously carried no `MT5_MCP_URL` and the
  compiled default was the only way to reach the terminal — a moved port was unfixable from the app.
- **Gates:** 10/10 both themes (verify_states, axe_audit, measure_render, lint_intent, design_lint),
  receipts in `.paul/ds-receipts/09-03/`. DOM measured: 5 `<li>` in an `<ol>`, badges all at x=37,
  uniform 14px gaps, no horizontal overflow at 320/560/768/1280, 0 console errors, preview-mode note
  still present. Screenshots: `.paul/ds-receipts/09-03/ui-{dark,light}.png` (not read as images this
  session — the model had no image input, so the look pass rests on the measured DOM and the gate
  receipts, per DEBT-016's pattern).
Status: done, design_lint red 0, yellow 0.

## 09-03: MCP check button + install progress bar (2026-11-27)

- Step 1 gained **Check address and token** and a determinate progress bar (`.progress`, `.bar`,
  `role=progressbar`, aria-valuemax=8 = the step count; `prog-fill` width = index/total).
- The bar is determinate on purpose: the step count is known, so an indeterminate spinner would lie.
- `aria-live` sits on the result line, not the bar: the screen reader hears "3 of 8: bridge.env"
  once per change instead of a percentage ticking every 400 ms poll.
- `prog-label`/`prog-detail` are only written when the text changed, so the poller cannot chatter.
- Bar measures 6 px on `--grid` with a 1 px `--line` border; the fill is `--accent` (dark: accent on
  grid, light: #b8761a on #f0ede8 — both far above 3:1 for a non-text indicator, WCAG 1.4.11).
- Bar resize measured at 320/560/1280: no horizontal overflow, fill width tracks percentage.
- Gates: 10/10 both themes after the fields were added; screenshots
  `.paul/ds-receipts/09-03/ui-{dark,light}.png` and the injected-progress shot
  `progress-dark.png` (13% → 50% → 100% widths verified in DOM).
Status: done, design_lint red 0, yellow 0.
