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
