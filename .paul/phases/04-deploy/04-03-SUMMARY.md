---
description: "04-03 apply record: privacy and notices pages, gate_all runner, docs and spec C2, Python retirement"
type: PlanSummary
plan: .paul/phases/04-deploy/04-03-PLAN.md
status: APPLIED
completed: 2026-11-25
---

# 04-03 SUMMARY: public content, the gate runner, docs, and the Python retirement

## What shipped

1. **`hub/privacy.html`** — the C13 outline in hub tokens: data held (Google account id, email,
   display name, session records; TTFC counts carry no identity), retention (sessions 30 days
   sliding, state backups 30 days, server backups 14 days), processors (Google sign-in, Cloudflare
   edge, Deriv/Bybit/Binance market data), account deletion (the DELETE endpoint and the
   foreign-key cascade), a contact line as a visibly marked placeholder (DEBT-020), the
   non-affiliation line, and a Last updated stamp. No scripts, no forms, no images, no hex
   literals (lint_hardcodes: zero), rem units only.
2. **`hub/notices.html`** — the AGPL-3.0-only source offer: /SOURCE.txt as the exact running
   version, the repository URL as a marked placeholder (DEBT-021), the THIRD_PARTY_NOTICES.md
   component table with licence links, the three attribution requirements, a comment naming
   THIRD_PARTY_NOTICES.md as the sync source.
3. **Licence texts served** — `hub/{Apache-2.0,MIT-lightweight-charts-drawing,NOTICE-vela,
   NOTICE-klinecharts}.txt`, byte-identical copies of `licenses/` (verified by diff), so every
   licence link on the page serves 200. The plan wrote `hub/licenses/`; that path is two segments
   and `static_files::resolve` serves one segment (or `vendor/`), and this plan changes no Rust,
   so the copies went flat into `hub/` — which is the "top-level files" the plan's own decision
   text described.
4. **Footer links** — Privacy and Notices from `hub.html`'s `#status` footer (styled from the
   existing tokens) and from `hub/index.html`.
5. **`.paul/runs/gate_all.py`** — one command for the design gate set against SERVED pages
   (DEBT-016 + DEBT-017). The kit's render gates hardcode `file://` (L-020), so the five browser
   gates are generated harnesses (never stored forks) that load the served URL through the kit's
   own Playwright (`channel: "chrome"`, the pattern check-0303 proves) and axe-core; the three
   static gates (lint_hardcodes, validate_tokens, design_lint) run as subprocesses. Receipts
   `gate.page.json` per run plus a manifest; `--check-receipts` fails on missing, stale, failed
   receipts **and on gate-list drift** between the script's enumeration and the manifest (which
   is what makes mutation m3 able to fail). First full run found three real defects on
   notices.html; all fixed (see Deviations).
6. **Docs** — README: the hosted truth (status, one port, gaffa/Procfile, ops/ pointer, the
   four-bucket rate-limit table, load-test line, launch requirements Q2/Q3/Q9 + public repo).
   SECURITY.md: the hosted threat model (tunnel edge with loopback-only CF-Connecting-IP trust,
   auth and CSRF, admin 404s, buckets, allow-list, hashed CSP with the named `'unsafe-eval'`
   residual, the anonymous TTFC write and its caps, secrets, redacted logs, Phase 7 SSRF),
   keeping the reporting section. AGENTS.md: the hosted-mode checks (one port, admin 404s, the
   three-guard test-idp build, served gate runner, bare exit statuses).
7. **Spec (C2)** — `## Risks and Mitigations` inserted as an H2 before Non-Functional
   Requirements: the PLANNING.md risk table adjusted to shipped reality plus the two new
   residuals (Pine compiler's `'unsafe-eval'`; single-host nightly-backup loss window). C19's H2
   promotion applied to Additional Resources, Future Improvements and Verification Plan.
8. **Python retirement** — the nine files `git rm`'d (hub/server.py, orderflow*.py x3, logbus,
   feedlog, capture_orderflow_fixtures.py, start.bat, start.sh). requirements.txt: comments only
   (bridge/mt5_bridge.py imports only the standard library). `hub/tests/` stays (owner's call).
9. **CI Rust-only** — the contract job lost setup-python, pip and the oracle server; it starts
   only hub-server on 8771 and runs the suite with `CONTRACT_RS_URL` alone. `contract.rs` no
   longer aborts without the oracle: the comparison cases skip with a count ("the Python oracle
   retired in phase 04") while the seven Rust-only probes still run against the Rust server or
   the in-process router. The summary step's skip grep matches the new wording.

## Task status

| Task | Status | Qualify |
|------|--------|---------|
| 1 privacy.html + footer links | PASS | lint_hardcodes zero; pages curl 200 from a live server; footer links grepped in hub.html and index.html; m1 verified |
| 2 notices.html + licence copies | PASS | every local link target 200; four `diff -q` byte-identical; m2 verified |
| 3 gate_all.py | PASS | 8/8 gates receipted on both pages; `--check-receipts` exit 0; m3 verified (twice, R-10) |
| 4 docs, spec C2, debt rows | PASS | debt store parses (016/017 done, 020/021 waiting); `^## Risks and Mitigations` count 1; H2 promotions in place |
| 5 retirement, ci.yml, mutations, SUMMARY | PASS | 9 files deleted; contract Rust-only 7 passed / 33 skipped / 0 failed, exit 0; all gates green; grep clean; m4 verified |
| checkpoint human-verify | agent-run | see below |

## Verification (all fresh this session, statuses read bare per R-8)

- `cargo fmt --all --check`: exit 0; clippy default and `--features test-idp`: no issues, exit 0 both
- `cargo test --workspace`: **139 passed**, exit 0; `cargo test -p hub-core --features test-idp`:
  **169 passed**, exit 0
- Contract suite Rust-only (`CONTRACT_RS_URL` alone, live server): **7 passed, 33 skipped
  (comparison cases), 0 failed**, exit 0 — the recorded count of oracle-relative cases
- `bash -n scripts/dev.sh`: exit 0; `npx -y js-yaml .github/workflows/ci.yml`: parses, exit 0
- `grep -rn "server.py" .github/ scripts/ Procfile README.md`: empty, exit 1
- gate_all: **8/8 gates receipted, 0 failing checks** on privacy.html and notices.html, served
  from a real hub-server; `--check-receipts` exit 0
- Mutations m1-m4 all red-then-green (m3 twice: the driver itself was edited after its first
  failed attempt, so the set re-ran against the restored file per R-10); log:
  `.paul/runs/mutate-0403.sh` run output

## Checkpoint (human-verify): verified by agent, delegated by owner

The blocking checkpoint ran in a real browser (Playwright MCP) against a live hub-server, per the
R-6 standing delegation:

- hub.html loads (only console entry: the pre-existing favicon 404, not CSP); the footer shows
  Privacy and Notices; clicking Privacy navigates to the page
- privacy.html renders fully in the hub theme: every AC-1 section present in the rendered text
  (data held, retention, processors, deletion cascade, marked contact placeholder,
  non-affiliation line)
- notices.html renders the licence table, the SOURCE.txt link, the marked repo placeholder and
  the three attribution requirements; screenshots in `.paul/runs/shots-0403/`
- What a script cannot judge is left to the owner as **DEBT-026** (the honest-content read) plus
  the already-open fill-ins DEBT-020 (contact) and DEBT-021 (repo URL)

## Deviations (auto-fixed, verified)

- **Licence copies flat, not `hub/licenses/`** (reason above; the diff proof is byte-identity).
- **The gate set is eight, not "seven kit gates plus design_lint" counted as 7/7:** the runner's
  enumerated list is authoritative (R-9), so the receipts and the flipped DEBT-016/017 checks
  expect `8/8 gates receipted`. The plan's AC-3 prose was internally inconsistent; the list wins.
- **axe injection needed `bypassCSP` in the audit context:** the pages' own (correct) CSP blocks
  the harness's injected axe script; only the audit context skips the policy, the site does not.
- **Three real defects found by the first gate run on notices.html, fixed:** the component table
  overflowed at 320/768 (now in an `overflow-x:auto` wrapper); a wrapped inline link's box centre
  probed its cell (cells' links now `white-space:nowrap`); axe could not run at all (above).
- **contract.rs oracle handling rewritten, not just CI:** without the oracle the whole suite used
  to run nothing; now the probes run Rust-only and the 33 comparison cases skip with a recorded
  count (scope limit respected: skipped, not deleted).
- **lab.js line 80 still says "start server.py"** in the feed-offline hud. `hub/lab.js` is
  04-02's surface, explicitly out of this plan's boundaries, and the AC's grep does not cover
  hub/. Left untouched, named here for the next plan that touches lab.js.

## Deferred rows (view of the store)

- DEBT-016, DEBT-017: **done** (gate_all.py is the diff; checks expect 8/8 receipted).
- DEBT-020 (privacy contact, Q9), DEBT-021 (repo URL), DEBT-026 (owner's honest-content read):
  waiting, owner.
- DEBT-025: waiting, agent at phase 05 — the CI browser-gate job (the narrowed remainder of
  DEBT-015's discharge).
- DEBT-022 (1,000-socket load test), DEBT-014 (semantic.text.link, owner), DEBT-018/019/023
  (host provisioning, drill, real backup), DEBT-024 (owner commit of the whole phase surface),
  DEBT-005/010/013: untouched, as the plan orders.

## Skill audit

| Skill | Expected | Invoked | Notes |
|-------|----------|---------|-------|
| /architect | plan | yes (plan stage) | SYNTHESIS.md sections 5 to 8 |
| /security-review (Phase 04 override) | per-task (Task 2, phase ship) | yes | loaded at APPLY (04-02); applied here to the AGPL offer (SOURCE.txt names the exact commit; no invented repo URL; the placeholder is a dashed, visibly unfilled block) and the retirement (the served tree shrinks; the allow-list still answers 404 to everything not listed; no new unauthenticated surface). No critical/high findings. |
| /modern-web-guidance | per-task (Tasks 1-2) | yes | guides `privacy`, `html`, `security` retrieved 2026-11-25 (64.5 KB, read in full) |
| /design-tokens | per-task | yes | both pages style only from theme.js variables; lint_hardcodes zero on both |
| /design-taste-frontend, /huashu-design, /a11y-audit | per-task | yes | rendered and read in the browser (screenshots); design_lint in gate_all; axe 0 violations both pages in gate_all; verify_states/responsive green (no covered focusables, no overflow) |
| /design-gate | unify | at UNIFY | gate_all.py receipt set is the runner |
| /technical-writing | optional | used as guidance | README/SECURITY rewritten plain; no separate invocation recorded |
| pstack | - | n/a | `pstack_stage: plan; APPLY ran in-session` |

### Modern Web Guidance

- `privacy`: data minimisation (the page lists exactly what the schema holds, nothing more),
  transparency by default (retention named in plain numbers), deletion as easy as sign-up (the
  header menu, no support contact needed), no dark patterns. Verbatim rule applied: "make it as
  easy to opt-out or delete an account as it was to sign up."
- `html`: doctype, `lang`, viewport, one `h1`, sequential heading levels, `main`/`footer`
  landmarks, links not buttons for navigation, `target="_blank"` always with `rel="noopener"`
  (lint_intent asserts all of it on the served page).
- `security`: frame-ancestors and the mandatory CSP directives are already the server's shipped
  policy (04-02); the new pages carry the strict default policy (no inline scripts, so nothing to
  hash). External links open with `rel="noopener"`.

## Reflect and RSI

- **Phase reflect ran at this UNIFY** (04-03 is the phase's last plan; `preferences.reflect: pstack`):
  a three-lens panel (judgment, tooling, divergent) over the digest in
  `.paul/runs/reflect-0400-digest.md`, then the synthesizer. Result: **11 accepted, 5 rejected,
  2 backlog**.
  - Close-out writes: `rules: +1` (R-11, the standing auto-approval boundary: it does not reach
    owner-owned rows, owner-judgment checkpoints, or irreversible acts); `decisions: +0`;
    `learnings: +5 added (L-032 subprocess encoding, L-033 channel chrome, L-034 Git-Bash server
    kill, L-035 static serving shape, L-036 js-yaml), 1 promoted (L-031, reviewer confirmation)`,
    `0 removed`.
  - `debt: +3 added` (DEBT-027 transcribe D-019..021 into DECISIONS.md; DEBT-028 re-create the 33
    comparison cases against recorded fixtures; DEBT-029 parameterize gate_all) `+1 note` (DEBT-025
    gains the dev-box window rule for CHECK_0303=1). R-9 gained the pages half; the uxui preset
    gained the gate-runner satisfaction note.
- unslop pass on this SUMMARY: no AI-prose patterns found; em dashes follow the repo's
  summary-family convention.
- rsi pass on the candidates: L-032..L-036 stay `candidate`. Each has one verified occasion, which
  justifies the rule but not a validated claim; they promote on their next independent hits or an
  owner's confirmation. No A/B campaign was launched: the standing approval does not cover a
  multi-run experiment budget at loop close.

## Next-phase readiness

Phase 04's build is complete after this plan. The phase exit still waits on owner rows: the
commit (DEBT-024), the host (DEBT-018), the real drill (DEBT-019), Q2/Q3/Q9 (020/021 + the
roadmap blockers), DEBT-022's load target, and the owner's page read (DEBT-026). Phase 05 (synced
state) plans against a stable, retired-Python tree with the served gate runner in place.
