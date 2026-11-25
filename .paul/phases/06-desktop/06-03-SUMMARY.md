# 06-03 SUMMARY

**Plan:** 06-03 — hub/auth.js desktop branch, the waiting overlay, the exchange and the refusal copy
**Applied:** 2026-11-25 (auto-continue from the session-reset marker; handoff 2026-11-25-6 consumed)
**Status:** COMPLETE — all 4 tasks executed, 33/33 driver assertions, 3 mutations red-then-green, receipts 8/8

## What shipped

| Task | Status | Qualify |
|---|---|---|
| 1. Isolate the sign-in start (`beginWebSignIn` / `beginDesktopSignIn`, `isDesktop()`, One Tap guarded by `if (!isDesktop()) loadGis(...)`) | DONE | PASS — node --check exit 0; AC-1 proven live (request #37 `GET /auth/google/start?client=web => 302`) |
| 2. The waiting overlay, four states (persistent card in `hub/auth.js`, token-only styles in `hub/hub.html`) | DONE | PASS after two defects found by the render-and-look pass (see deviations) |
| 3. The exchange and the refusal copy (module-scope listener, per-status copy, key burned every path) | DONE | PASS — 401 proven against the real server route; 400/503/throw via driver routes |
| 4. The driver, the receipts, the a11y pass (`.paul/runs/check-0603.mjs` + `.sh`, 12 cases) | DONE | PASS — 33/33, 12/12 cases; 3 mutations; 8/8 receipts; check-0503 22/22 |

The driver lives in `.paul/runs/` (gitignored, like `check-0503.mjs` and the other drivers — the
established convention), launched by `bash .paul/runs/check-0603.sh` (builds unless `SKIP_BUILD=1`,
server on 8774, PID-tracked cleanup per the check-0402 pattern).

## Verification (all bare, R-8)

| Command | Result |
|---|---|
| `node --check hub/auth.js` | exit=0 |
| `bash .paul/runs/check-0603.sh` | exit=0 — 33/33 assertions, 12/12 cases (L-026 count asserted) |
| mutation (a): 401 copy → 400 copy | red exit=1, exactly case 7 fails; reverted green exit=0 |
| mutation (b): double-click guard off (`if (false) return;`) | red exit=1, exactly case 3 fails (calls=2); reverted green exit=0 |
| mutation (c): `aria-busy` dropped from waiting | red exit=1, exactly case 2 fails (`status/polite/null`); reverted green exit=0 |
| `python .paul/runs/gate_all.py --base http://127.0.0.1:8775 --pages hub.html,index.html` | exit=0 — 8/8 gates, 2 pages, 0 failing checks |
| `python .paul/runs/gate_all.py --check-receipts` | exit=0 — 8/8 receipted, no stale |
| `CHECK_HOSTED=http://127.0.0.1:8773 node .paul/runs/check-0503.mjs` | exit=0 — 22/22 (the phase-05 sync surface is unchanged) |
| DEBT-035 negative control (`grep -c 'auth-off' .paul/runs/gate_all.py`) | exit=1 — correctly not discharged |
| Served browser pass (playwright MCP on the ad-hoc test-idp server) | AC-1 navigation, AC-2 one call/no nav, cancel restore, 401 copy — all observed live; screenshots in `.paul/runs/shots-0603/` |

## AC results

| AC | Evidence |
|---|---|
| AC-1 web path unchanged | check-0503 22/22; driver case 1 (client=web requested, no exchange, no startSignIn); the click handler body moved verbatim into `beginWebSignIn` |
| AC-2 one command, no nav, double-click guarded | driver cases 2-3; live pass (pointer-events:none via aria-busy plus the `deskBusy` guard) |
| AC-3 four states, announced, escapable | driver cases 2 (waiting), 4 (cancel), 5 (60 s timeout via `page.clock.fastForward`), 6 (200 → reload → avatar); focus moves in and returns |
| AC-4 each refusal its own copy | driver cases 7-10 assert the full string and the absence of the other three; 401 also proven against the real route |
| AC-5 clean page and receipts | gate_all 8/8 with `--check-receipts` exit 0; `lint_hardcodes`/`validate_tokens` green (overlay styles are `var(--…)` only) |

## Auto-fixed defects (all found by the render-and-look / served pass, not by the unit checks)

1. **`.desk-card .steps` rendered in the waiting state.** `display:grid` on the element beats the UA
   `[hidden] { display:none }` rule, so the steps list showed under the waiting copy while the
   programmatic check reported `hidden=true`. Fixed with `.desk-card .steps[hidden] { display:none }`.
   Screenshot evidence: `waiting-state.png` (broken) vs `waiting-state-fixed.png`.
2. **Card heading was an `h3` and axe failed `heading-order`.** The page's only heading is
   `<h1>TradeX</h1>`; the card's `h3` skipped `h2`. Demoted to `<h2>` (the mockup's `h3` was
   mock-context). Driver case 12 green after.
3. **`deskHide`'s parameter shadowed `render()`.** `function deskHide(render)` made the call
   `render()` invoke the boolean — Cancel removed the overlay but left the busy button and lost
   focus. Renamed to `rerender`; the driver's `pageerror` assertion now guards this class of bug.

## Decisions made in APPLY (not D-NN: implementation-level)

- **Overlay DOM home:** the fixed-position card is appended to `document.body` (like `failedNotice`),
  not into `#authSlot` — the plan's "into the existing header slot area" names the visual anchor (a
  fixed card over the app), and body-append avoids any ancestor clipping question.
- **`startSignIn` rejection copy:** the plan required a "try again from the app" failed state without
  giving the string; shipped as `Error: the app could not open your browser. Nothing was signed in.
  Choose Try again, or restart the app.` (the copy discipline's shape).
- **Unexpected exchange status (not 401/400/503):** mapped to the 503 "server not available" copy
  rather than an unhandled state; the server contract is 200/401/400/503, so this branch is defensive.
- **Token mapping:** the mock's `--dialog-w` card maps to the hub's `--dlg-w` (440 px small-dialog
  token); the mock's raw `2px` icon offset maps to `--badge-ps`. No new token needed; no new colour.
- **Gate receipts base:** the 8-gate run used an auth-off hub (`--base http://127.0.0.1:8775`), the
  condition the receipt convention was built on (gate_all's default 8770 dev hub is auth-off). With
  auth on, the D-019 sticky auth slot hit-tests as covering the bar content that scrolls under it
  (1/4/1 covered focusables at 320/768/1280 px) — diagnosed live, recorded as DEBT-035.

## Deferred rows

- **DEBT-035** (new, agent, phase:06): gate_all.py must name its auth-off base requirement; check =
  `grep -c 'auth-off' .paul/runs/gate_all.py` expect 1 (negative control run: fails today).
- DEBT-033's check now passes (`D-026` is in DECISIONS.md) — its `status = "done"` flip is 06-04's,
  per the phase-06 handoff. DEBT-034 (the `/api/me/state` docs page) is also 06-04's.

## Modern Web Guidance

- `accessibility`: the waiting state's status text stands alone out of context ("Finish signing in in
  your browser…"), and the guide's rule that live regions are noise for interstitial states is why the
  waiting copy names the place instead of saying "Loading" — quote: "Don't use live regions for
  interstitial states like 'Loading…' or 'Updating…' unless they're meaningfully informative — they
  usually just create noise." This one is meaningfully informative: the flow spans two applications
  and the user must look away from the app to finish it.
- `accessible-error-announcement`: the four states swap inside ONE persistent card, because a shown
  live region is not reliably announced — quote: "a live region that is shown, rather than changed,
  isn't announced reliably." The failed state switches to `role="alert"` (implicit assertive),
  removes the explicit `aria-live="polite"` (an explicit value would override the role's implicit
  urgency) and clears `aria-busy` — quote: "`aria-busy` must not stay `true` over a finished state"
  (synchronise visual and programmatic state). Focus moves to the heading on show and to Try again
  on failure, and returns to the sign-in button on leave (the window has no other chrome).

## Skill audit

| Skill | Invoked | Notes |
|---|---|---|
| context-budget, caveman, ponytail | yes | loaded at session start (AGENTS.md rule) |
| /design-doctrine, /design-component, /design-code | yes | kit SKILL.md files read in full before Task 2 (the plan's BLOCKING gate) |
| /modern-web-guidance | yes | `retrieve "accessibility,accessible-error-announcement"` run before Task 2; both applied (section above) |
| /a11y-audit | yes | driver case 12: axe 0 violations, waiting and failed, before the plan was declared done |
| /design-taste-frontend, /huashu-design | via gate run | gate-runner satisfaction note (SPECIAL-FLOWS): receipt set + served browser pass with screenshots; entry added to `.paul/design/polish-log.md` |
| /design-tokens | n/a | no new token; existing scale covered every value |
| /design-gate | unify row | receipts refreshed here; the gate evidence runs at UNIFY |
| pstack | in-session | `pstack_stage: plan; APPLY ran in-session` — poteto-mode/architect not gated in APPLY |

## Issues encountered

- Obscura (the config browser) refuses loopback URLs (SSRF guard); the served passes ran on the
  playwright MCP instead. Recorded for the phase's browser-tooling note.
- The ad-hoc auth-on gate run produced 7 failing checks: 2 real-diagnosis classes (covered focusables
  from the sticky slot; networkidle timeouts on the index.html meta-refresh stub) — all resolved by
  running the receipts under their auth-off convention; no page defect behind them.
- `VTV_SECRET_KEY` must be 48-byte base64 (a short literal is rejected at startup).

## Next-phase readiness

- 06-04 (CI job for the desktop crate, unsigned installer builds, `docs/desktop.md` + the
  `/api/me/state` docs page, the DEBT-033/034 rows) is the phase's last plan; its human-verify
  checkpoint (real sign-in from the installed app) can now pass — DEBT-013's URI is registered and
  06-03's page branch is what completes the flow.
- The owner's no-deploy decision stands: the live site stays v0.1.1; the 06-03 page change is
  verified on the test server only.
