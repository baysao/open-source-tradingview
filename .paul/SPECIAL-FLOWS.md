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

---

## Amendment History

| Date | Change | Reason |
|------|--------|--------|
| 2026-11-23 | uxui kit preset written (10 rows), phase overrides 03, 04, 06, 07, Session Rules R-4 and R-5 | /seed launch: design track D3 done for a rendered surface |
| 2026-11-23 | pstack preset written for stage `plan` (architect, interrogate, swarm, technical-writing, paul-security, cstack-reflect, reflect); `poteto-mode` and per-task `architect` not written (stage plan); `show-me-your-work` dropped (not installed); pstack `design-taste-frontend` row dropped (the uxui row already owns it as required) | via /paul-config: mode full |
