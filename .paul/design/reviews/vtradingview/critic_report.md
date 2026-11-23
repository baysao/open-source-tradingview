# Critic Report (Stage 7, round 1): vtradingview

*   **Target:** `.paul/design/specs/vtradingview.md`, whole file (FeatureRange 1 to 914)
*   **Reviewers (direct dispatch, parallel, fresh context, no GoldfishContext):**
    *   `egm-design`: `feature-spec-standard` + `visual-design-standard`, completed
    *   `egm-structural`: `feature-spec-standard`, completed
    *   `egm-style`: persona vectors, completed
*   **Raw totals:** design 1 Blocker, 15 Major, 6 Nit; structural 1 Blocker, 3 Major, 7 Nit; style 0 Blocker,
    9 Major, 23 Nit.
*   **Date:** 2026-11-23

Elephant checks: the CSRF Blocker is confirmed (spec line 295 returns `csrf_token` only at sign-in,
line 718 keeps it in memory only, and the database holds only its hash). The polish-log inconsistency is
confirmed (`polish-log.md` line 61 keeps the subagent's old "red 2 accepted" status under Screen 05,
superseded by the orchestrator round at line 9). The Stage 5 and 6 findings that recur here are marked.

## Consolidated findings

Ids: `D` design, `S` structural, `T` style. Findings repeated across reviewers are merged and keep all ids.

### Blockers

| Id | Finding | Scope |
|---|---|---|
| C1 (D1) | **CSRF token is lost on reload.** Returned only at sign-in, kept in memory, stored hashed; a reloaded page or second tab cannot obtain it, so every write (sync, logout, delete, MT5, admin) fails until a new sign-in. Breaks G4 and CUJ 4. Specify how a loaded session gets a token (`/auth/me` issues one; several per session for tabs; or a double-submit cookie) and add reload cases to the tests. | 293-300, 317-322, 718 |
| C2 (S1) | **No Risks & Mitigations section.** The standard requires it; risk material is scattered over Open Questions, the Technical Plan and an NFR line that defers to the plan; the only risk table is in PLANNING.md. | between 427 and 526 |

### Majors: document defects

| Id | Finding | Scope |
|---|---|---|
| C3 (D2, S2, T1; Stage 5 F7, Stage 6 F1) | **Merge rule both open and decided.** Close Q8, rewrite CUJ 3. Raised by five independent readers. | 107-110, 136-139, 324, 493-500 |
| C4 (D3) | **Screens not linked to the mockups.** One row of state names per screen; no layout, anatomy or placement; the D4 files exist but are not linked. Link each row to its mockup and make the mockup normative. | 393-425 |
| C5 (D4) | **Theme source incomplete.** Variable list omits `--onAccent` and the 12/14 px size tokens, which live only in the mockups' `shared.css`; no plan item creates the product stylesheet; `design-tokens.json` and `tokens.json` unexplained. | 395-400, plan |
| C6 (D5) | **Interactive states unspecified.** No hover, focus, pressed, disabled per control; no focus-ring token; no accent hover shade; no accessible names (icon-only avatar, restart, Restore). | 413-421 |
| C7 (D6) | **Data and failure states missing.** Sign-in failure, avatar fallback, session expired (401), over-cap value, 256-key limit hit during merge, MT5 list loading and degraded bridge, admin loading, empty, disconnected, stale, too-few-samples. | 415-420 |
| C8 (D7) | **Admin panel lacks the Dashboard Rule.** No question, numbers with unit and baseline, numeric problem rules, action per problem, or known-totals fixture. | 183-189, 363-368, 419 |
| C9 (D8, D9) | **Gates misnamed and not runnable as written.** `accuracy_report.mjs` is the kit's self-test, not project evidence; taste gate and token-layer checks not named; no production-build rule or FAIL/WARN policy; three presets and five fonts cannot be forced by a `--dark` flag; no harness page named for built screens. | 423-425, 901-904 |
| C10 (D10) | **Contrast rows dropped from the source.** Light `dim` on `raised` 4.46, light `ok` on panel 4.45, light accent on panel 3.39 (taste/hub-theme.md) are not carried; destructive style does not say fill or border or name the label colour. | 401-411, 418 |
| C11 (D11) | **Google UI assumptions.** FedCM prompt position is browser-controlled; `auto_select: true` signs in with no click (contradicts "one click") and triggers the merge unasked; a themed Google button conflicts with Google branding rules. | 119-130, 270-272 |
| C12 (D12) | **No phone layouts**; the known mobile URL-wrap defect from the polish log is not carried. | 413-425 |
| C13 (D13) | **No copy deck**, and no privacy-page content outline (data held, retention, processors, contact). Privacy content should be a launch blocker with Q2 and Q3. | 413-421, 765-768, 528-531 |
| C14 (D14) | **Global feed cap exhaustible by about seven IPs** (4 x 16 = 64 keys per IP vs 400): new visitors may not get the default chart. Define what the cap counts, reserve default keys, lower anonymous budget, check provider subscribe-rate limits. | 245-250, Q6 |
| C15 (D15) | **4 sockets per IP** breaks shared NAT and multi-tab users; sockets per tab not stated. | 246 |
| C16 (D16, T2; Stage 6 F12) | **G2 cannot be verified as written.** The in-memory histogram is lost on deploy; the beacon is unauthenticated and can be skewed; the metric has four names and two end events. Define the metric once and persist the histogram or redefine the window. | 59, 119, 366-368, 762, 905 |
| C17 (D17) | **Motion decision handed to the spec and not made.** | 393-425 |
| C18 (S4, T9) | **Rejections rest on "the owner chose".** Lead each Reason for Rejection with the technical or cost reason; B and I give none. | 427-521 |
| C19 (S3, T31) | **Heading hierarchy.** Additional Resources and Future Improvements are H3 under NFR; Verification Plan reads as an eighth phase. | 541-559, 886 |
| C20 (T3) | **"Measured in this session"** has no reproducible source; link the measurement record. | 400-401 |
| C21 (T4; Stage 6 F5) | **"D4" and "the lint" undefined.** | 398-400 |
| C22 (T5; Stage 6 F8) | **"Shared constants" crate unnamed.** | 223 |
| C23 (T6; Stage 6 F15) | **Bars cache TTL has no value.** | 252 |
| C24 (T7) | **"The day pgsqlite misbehaves" is not a testable trigger.** | 479-480 |
| C25 (T8) | **Load test "realistic subscriptions" undefined.** | 772-773, 533 |

### Nits (grouped)

*   **Definitions and jargon:** seed, draw, dealt, gaffa, pricehub (T10, Stage 6 F2, F6); "treated as local"
    (T18, Stage 6 F10); "repo rule" source (T24); reference template used before introduced (Stage 6 F9).
*   **Precision:** "real" as intensifier (T11); "per need" (T12); filler framings (T13, T14, T20, T25);
    CUJ 1 percentile on one visit (T15); One Tap corner (T16, overlaps C11); "No second router exists
    anywhere" (T17); time-relative references "as today", "this branch" (T19); subjective "acceptable",
    "cheap" (T21); "amber" vs user-overridable `--accent` (T22); ratios not tied to presets, "about 1.3"
    (T23); unverified claims incl. "Vela may hold dozens of keys" vs Q5 (T26); pinned versions listed only
    for three crates (T27); "CI and dev build alike" (T28).
*   **Structure and format:** header names vs standard (S6); mixed C/M/D markers in items 35 and 52 (S8);
    over-long lines 368, 762 and double blank line (S9, T30); unresolvable `~/` paths in resources (S10);
    ambiguous `auth.rs` (S11, T29); alternatives F and G break the Pros/Cons shape (S5); provenance footer
    (T32).
*   **Design details:** no explicit statement that Idea, Argument and Imagery rules do not apply (D18);
    polish-log Screen 05 status contradicts the index (D19); "chart UI does not change" overstated (D20);
    socket not reopened on sign-in (D21); merge "values equal" needs canonical JSON compare (D22); Try
    again and early deep link in desktop flow (D23); up to 24 h backup loss not stated as accepted (D24).

## Carried from Stages 5 and 6 (not re-raised here, still open)

*   Stage 5 F1, F3, F4, F5: owner decisions challenged (Rust rewrite vs async Python, two engines, Tauri vs
    PWA, MT5 bridges in first release). Record cost against need.
*   Stage 5 F2 and Stage 6 F3: retired self-hosted product, Local MT5, SSH Remote MT5, changed `ws_port`,
    `mt5`, `arm` fields.
*   Stage 6 F4: how the desktop page obtains `desktop_key` (design gap).
*   Stage 5 F12: AGPL-3.0 source offer on the hosted site.
*   Stage 5 F6, F8, F9, F10, F11 and Stage 6 F7, F11, F13, F14, F16: Nits.

## Round status

Not clean: 2 Blockers and 23 consolidated Majors open. Under `egm-review-loop` rules the next step is
resolution (`egm-resolve`), then a re-audit (max 1 at this stage).
