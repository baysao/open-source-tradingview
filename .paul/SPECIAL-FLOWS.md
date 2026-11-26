# Specialized Flows

**Project:** vtradingview
**Created:** 2026-11-23
**Last Updated:** 2026-11-23 (mode full)

---

## Project-Level Dependencies

| Work Type | Skill/Command | Priority | Checked | When Required | Notes |
|-----------|---------------|----------|---------|---------------|-------|
| design | /design-doctrine | required | per-task | first design/UI session of any phase | uxui kit preset |
| design | /design-tokens | required | per-task | creating or changing a palette, type scale, spacing or theme (`design-tokens.json`, `hub/theme.js`) | uxui kit preset |
| design | /design-component | required | per-task | speccing a new component before code | uxui kit preset |
| frontend | /design-code | required | per-task | implementing component code | uxui kit preset |
| frontend | /a11y-audit | required | per-task | every UI phase, before exit | uxui kit preset |
| frontend | /design-gate | required | unify | UNIFY evidence on every UI phase | uxui kit preset |
| frontend | /design-taste-frontend | required | per-task | build and render gate of a page or app screen (direction fixed: `hub-native`) | uxui kit preset |
| frontend | /dashboard-design | required | per-task | the admin log panel (data screen): question, numbers, problem rules, known totals | uxui kit preset |
| frontend | /modern-web-guidance | required | per-task | any task writing HTML, CSS or browser JS | uxui kit preset |
| frontend | /huashu-design | required | per-task | render-and-look on every screen built or changed: `polish.py shot`, `design_lint.py check` (red fixed), logged in `.paul/design/polish-log.md` | uxui kit preset |

**Gate-runner satisfaction (phase 04 reflect, 2026-11-25):** a per-task uxui row whose surface is covered
by the current `.paul/runs/gate_all.py` receipt set plus the served-page browser pass is satisfied by that
run; invoke the skill separately only for surfaces outside the receipt page list. (04-03 satisfied
design-taste-frontend, huashu and a11y-audit this way; design-gate stays a UNIFY row and runs the receipts.)
| planning | /architect | required | plan | plan sketch: types, signatures, module boundaries for any plan with code crossing a function boundary | pstack preset |
| planning | /interrogate | optional | plan | contested design in the plan, pre-ship review | pstack preset |
| planning | /swarm | optional | plan | research and coverage matrices while planning | pstack preset |
| docs | /technical-writing | optional | per-task | docs, RFCs, PR descriptions | pstack preset |
| security | /paul-security | optional when `security_audit.enabled: true` | per-task | task touches auth, input parsing, a new endpoint, a path, or a dependency; or a phase ships | pstack preset |
| close-out | /cstack-reflect | required when `reflect: cstack` | unify | UNIFY of each phase's last plan (then `rsi`) | pstack preset |
| close-out | /reflect | required when `reflect: pstack` | unify | UNIFY close-out, once per phase (then `rsi` handoff); user-only in Claude Code: type `/reflect` | pstack preset |

**Priority Legend:**
- `required` - enforced at its Checked moment (`per-task` when a matching task starts, `unify` prompted at
  UNIFY, declined = `deferred`, not a gap).
- `optional` - informational only, no gap logged.

---

## Phase Overrides

| Phase | Additional Skills | Notes |
|-------|-------------------|-------|
| 03 | /security-review | sign-in, sessions, CSRF, admin gate |
| 04 | /security-review | CSP, rate limits, tunnel, AGPL notice |
| 06 | /configuring-tauri-capabilities, /signing-tauri-apps | remote origin gets no IPC beyond sign-in |
| 07 | /security-review | SSRF guard and bridge tokens |

---

## Templates & Assets

| Asset Type | Location | When Used |
|------------|----------|-----------|
| Design doc | `.paul/design/specs/vtradingview.md` | every plan |
| Mockups (normative layout) | `.paul/design/opendesign/vtradingview/` | UI tasks in Phases 03 to 07 |
| Theme record | `.paul/design/taste/hub-theme.md` | any colour or type decision |
| Review findings | `.paul/design/reviews/vtradingview/` | phase gates |

---

## Session Rules

- **R-4: deferred work lives in `.paul/DEBT.toml`,** one `[[debt]]` row per item, each with a check that
  can fail. Rationale: a STATE bullet or prose note cannot fail, so it is never revisited.
- **R-5: evaluate the store at the gates the loop already passes:** session entry
  (`debt.py --run-checks`; an `overdue` blocking row is the next action), plan drafting (an uncovered row
  is a plan defect), phase exit (`--gate phase-NN-exit`), loop close (discharge is a TOML diff).
  Rationale: a store nobody is told to read is a file, not a scheduler.
- **R-6: human-verify checkpoints are agent-run in a browser first.** When the steps can be observed in a
  browser, the agent runs them with the browser MCP, fixes what it finds, records the result as `verified by
  agent, delegated by owner`, and leaves one residual owner `uat` row for what a script cannot judge.
  Rationale: the owner set it as standing ("use playwright self check/verify", confirmed at the Phase 01
  reflect); in 01-03 the browser pass found a defect that 55 tests and the contract suite missed.
- **R-7: probe a runtime Fact with one command before any task builds evidence on it.** A route, a feature
  flag's effect, a mount, an env var: the Fact is a hypothesis until curl/grep says otherwise. A disproved
  Fact becomes a recorded deviation with the smallest fix, not a surprise at the checkpoint.
  Rationale: 03-03's plan assumed `hub-server --features test-idp` served `/__test-idp/*`; the feature
  compiled the code but `app()` mounts that router only for an in-memory Idp, so the whole browser check
  had no IdP (`/__test-idp/jwks` 404). The premise was one curl away the entire time.
- **R-8: quote the exit status of the command you judged, bare.** A pipeline's status is its last stage's,
  so `cargo fmt --check | tail` reports `tail`'s 0. Pass/fail commands run bare, under `set -o pipefail`,
  or followed by `; echo exit=$?`. Pipes are for logs, never for the command whose status is the claim.
  Rationale: 03-03 claimed "fmt clean" from a tailed command; `cargo fmt --all --check` was in fact
  failing on the new helper.
- **R-9: an incomplete gate set is an unrun gate.** Enumerate the project's gates, run all of them, and
  treat a missing receipt as a failure. A gate named in a document but absent from the run is unverified
  and must not be cited. The enumeration names the gates **and the pages each gate must cover**: "8/8
  gates receipted" over a page list that excludes the touched surface is the same failure.
  Rationale: 03-03 cited "CI's `cargo tree` assertion" that `ci.yml` never had, and ran the five-gate
  design set as four, skipping exactly `lint_hardcodes`/`validate_tokens`. Phase 04 added the pages half:
  gate_all stamped 8/8 over only its birth plan's two pages while the hub.html/index.html footer edits
  got a grep and one browser look.
- **R-10: editing a checker invalidates its mutation results.** After any edit to a check, a driver or a
  gate, re-run the mutation set before claiming coverage. Tuning a check until green proves agreement,
  not correctness.
  Rationale: 03-03's browser driver was edited twice to green; the four AC-5 mutations were then re-run
  against the final script at UNIFY, which is the only reason the claim holds.
- **R-11: the owner's standing approval ("auto yes, auto approve, do not ask if not risk", given mid-phase 04) covers routine confirmations only.** It does not reach owner-owned debt rows (a row whose evidence lives in a file the agent writes discharges only on evidence the owner provides), owner-judgment checkpoints (the honest-content eye), or irreversible acts outside an approved plan. When such a case appears, ask; everything else proceeds without asking.
  Rationale: the directive delegates the asking decision to the agent; without a written boundary the next
  session either re-asks through every gate or self-discharges the rows meant to catch what a script cannot
  judge (04 reflect: DEBT-020/021/026 are `text_present` on a SUMMARY the agent authors).
- **R-12: an automated falsifier that can reopen a phase runs before the phase's human-judgment gate closes it.** When a push is deferred (`auto_commit: false`), the phase exit records "CI-unproven on this tree" as an open exit condition in the phase's own record — the first-push observation (the DEBT-038 pattern) belongs to the phase's gate arithmetic, not to a post-exit artifact row that only archaeology can act on. If the first push can fail the phase's build, a "closed" phase reopens; order the cheap automated signal before the expensive human gate whenever the automated result can contradict what the human gate approved.
  Rationale: 07 reflect: phase 07's exit was gated on DEBT-039 (the owner's real-bridge observation) while the only signal that could falsify the phase (first-push ci.yml/desktop.yml green) was deliberately sequenced after exit — CI was being used as archaeology instead of as a gate.

---

## Amendment History

| Date | Change | Reason |
|------|--------|--------|
| 2026-11-23 | uxui kit preset written (10 rows), phase overrides 03, 04, 06, 07, Session Rules R-4 and R-5 | /seed launch: design track D3 done for a rendered surface |
| 2026-11-24 | Session Rules R-7 (probe a runtime Fact), R-8 (bare exit status), R-9 (incomplete gate set), R-10 (re-run mutations after editing a checker) | 03-03 UNIFY reflect: 4 accepted findings, all with direct evidence in the session |
| 2026-11-25 | R-11 (standing auto-approval boundary), R-9 gains the pages half, uxui gate-runner satisfaction note | phase 04 reflect at 04-03 UNIFY: 3-lens panel + synthesizer, 11 accepted, 5 rejected, 2 backlog |
| 2026-11-23 | pstack preset written for stage `plan` (architect, interrogate, swarm, technical-writing, paul-security, cstack-reflect, reflect); `poteto-mode` and per-task `architect` not written (stage plan); `show-me-your-work` dropped (not installed); pstack `design-taste-frontend` row dropped (the uxui row already owns it as required) | via /paul-config: mode full |
