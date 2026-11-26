# 07-04 SUMMARY: settings screen, picker bridge wiring, gate receipts, served driver, owner checkpoint

**Applied:** 2026-11-26 (session-reset auto-continue from 07-03's pause; sequential in-session, pstack_stage: plan).
**Status:** UNIFIED 2026-11-26 — checkpoint approved; DEBT-039 discharged 2026-11-26 (the owner observed the real bridge streaming live XAUUSDm candles; the transition gate cleared).
**Plan:** `.paul/phases/07-bridges/07-04-PLAN.md` · **Contract:** `.paul/phases/07-bridges/07-ARCHITECTURE.md`

## Tasks
| # | Task | Status |
|---|------|--------|
| 1 | `hub/settings.html` + `hub/settings.js` from mockup 03 (six states, guide-conformant form) | done |
| 2 | Avatar menu entry ("MT5 bridges" → `/settings.html`) + picker/subscribe bridge wiring (lab.js) | done |
| 3 | Gate receipts with settings.html in the page list + served driver check-0704 + a11y mutation | done |
| 4 | checkpoint:human-verify — the real-bridge half (DEBT-039) | approved; DEBT-039 stays with the owner |

## Acceptance criteria
| AC | Result |
|---|---|
| AC-1 six states in the hub theme | green (mockup 03 ported: empty, list + inline delete confirm, add form, testing busy, connected with symbol count, error with the exact cause; theme variables only — lint_hardcodes/validate_tokens/design_lint receipts green) |
| AC-2 guide-conformant form | green (driver c3/c4/c5/c12: errors defer to submit with focus routing, aria-invalid false until user-invalid, submit disabled only after a valid submit, one polite live region announces test/delete results; the mutation that breaks the aria-invalid sync fails the probe red, restored green) |
| AC-3 menu entry + picker + bridge field | green (auth.js `item("bridges", "MT5 bridges")` signed-in only; merged entries normalized `ticker=name`, group `mt5-bridge`; lightweight.html optgroup "MT5 bridge"; driver c9/c10: picker lists XAUUSD/EURUSD, ws payload `{"source":"mt5","symbol":"XAUUSD","tf":"M1","bridge":"local"}` with 2 real bars through the fake's `/wait`) |
| AC-4 gate set receipts the page | green (`gate_all.py` 8/8 gates × 5 pages on the auth-off base `http://127.0.0.1:18774`, settings.html in the list; `--check-receipts` exit 0; axe 0 non-minor (driver c11); occlusion covered=0 at the harness widths; polish-log entry written) |
| AC-5 driver proves the client, owner sees the real thing | green for the client half (14/14 cases, mutation red-then-green inside the final run); the real-bridge half is DEBT-039, owner-owned, approved as a checkpoint but not yet observed |

## Verification
- `node .paul/runs/check-0704.mjs` (via `bash .paul/runs/check-0704.sh`, SKIP_BUILD=1): **14/14 cases, CHECK-0704-PASS, exit 0** (bare); c12 mutation red (`aria-invalid=true`) then green (`false`) inside the final script (R-10).
- `python .paul/runs/gate_all.py --base http://127.0.0.1:18774 --pages hub.html,index.html,settings.html,privacy.html,notices.html`: **8/8 gates receipted, 0 failing checks, exit 0**; `--check-receipts`: **OK, exit 0**.
- `justrust test --workspace`: **313 passed, 0 failed** across 11 suites, exit 0 (bare; `justrust show 20261126-093121272-21796`).
- `debt.py --run-checks`: **overdue=0, waiting=13, done=26** — DEBT-039 waiting (event:owner-bridge-live, blocks phase-07-exit).
- Gates ran on a fresh ephemeral hub (18774, auth-off base per DEBT-035), not the pre-existing dev hub on 8770.

## Deviations (each with its reason)
1. **files_modified over-listed the engine pages**: `hub/vela.html`, `hub/lightweight.html`, `hub/klinechart.html` have no ws send site of their own — all three call `LabFeed.live/history`, so the one send site is `lab.js`. lightweight.html was still touched, but for the picker (the `mt5-bridge` optgroup + label), not its subscribe.
2. **The merged bridge entries carry no `ticker`** (server sends the MT5 row verbatim: `name/desc/path/digits`), so `lab.js` normalizes once at load: `ticker = name`, `group = "mt5-bridge"`. Client-only; the anonymous `/api/symbols` shape is untouched (G1).
3. **`history()` also needs the bridge param**: `/api/bars` routes to the bridge path only when `bridge=` is present (market.rs); `source=mt5` alone stays the D-009 Off feed. lab.js appends `&bridge=<id>` from the same entry lookup.
4. **The register UI cannot be exercised live on the auth-off hub**: `/api/me/mt5` is hosted-only (routes/table.rs:49), so driver c8 runs the flow against the route mock (201); the live register/404/401 halves stay with `bridge_routes` and check-0703's served cases. The D-033 seam (bridge `local`) carries the live bars/ws proof instead.
5. **`/auth/me` first**: the page's boot `/api/me/mt5` fetch 404'd signed-out — a console error on every visit (design_lint red) and the real UX bug that local mode showed "server refused". Now signed-out/local shows the sign-in panel and never touches the mt5 surface. Driver mocks `/auth/me` auth:true for the signed-in cases.
6. **An unread error body kept the boot request alive forever** (headers resolved, body stream never finished; networkidle never settled — every browser gate timed out on the page). `loadList` consumes the body on every path (`r.text()`), which is also the correct client behaviour.
7. **Mockup's "Save bridge" became "Done"**: the contract's `POST /api/me/mt5` registers AND tests in one call (201 = saved), so a separate save button would lie. Connected state shows "Done" (primary) + "Test again".
8. **Small style debts fixed en route**: 4 hardcoded px → rem (lint_hardcodes); wrapped-inline-link occlusion → `a.link { white-space: nowrap }` (verify_states covered=1→0); footer added (lint_intent hasFooter).

## Skill audit
| Skill | Loaded? | Note |
|---|---|---|
| context-budget, caveman, ponytail | yes (session) | AGENTS.md rule |
| /architect | yes (plan) | 07-ARCHITECTURE.md is the sketch |
| /security-review | deferred to phase exit | the phase override runs `--full` under the transition gate (07-03's reading) |
| /paul-security | not invoked | subsumed by the phase override (07-01's reading) |
| /design-doctrine, /design-component, /design-code | yes | mockup 03 specced before code; DESIGN.md rules applied |
| /design-taste-frontend, /huashu-design, /a11y-audit | yes | satisfied via the gate-runner note (gate_all receipts incl. settings.html + served browser pass), the phase-04 reflect precedent; polish-log entry written |
| /modern-web-guidance | yes (plan) | forms, validate-input-after-interaction, accessible-error-announcement applied; no new capability retrieved at APPLY |
| /design-gate | unify | the receipted gate run is the runner |
| pstack | pstack_stage: plan | APPLY ran in-session; no poteto-mode routing |
| parallel_agents: true | degraded to sequential | ZCode has no worktree isolation (the wave-1 gotcha) |

### Modern Web Guidance
- **forms**: the add form is a real `<form method="post" action="/api/me/mt5" novalidate aria-labelledby="formH">` with a `name` on every control, visible stacked `<label for>`s, persistent hints, and `required` marked visually and by attribute. The CSP's `form-action 'none'` means the submit handler always `preventDefault()`s and fetches — the method/action name the intent, the fetch does the work.
- **validate-input-after-interaction**: "native constraints (`required`, `pattern`) with `:user-invalid` styling so the error state defers until the user commits (blur or submit)" — errors clear on `input` when valid, validate on `blur` (if edited), block and route focus on `submit`; the submit button disables only after a valid submit.
- **accessible-error-announcement**: `aria-invalid="false"` from load (the mutation proves the probe), the error id joins `aria-describedby` only while invalid, and one polite live region (`#live`) announces test/delete results.
- **persistent-toast-notifications** (rejected at plan time, upheld): results render inline per mockup 03; no popover dependency entered.

## Deferred rows
- **DEBT-039** (existing, check repaired at this unify): the owner's real-bridge observation — the exact line "DEBT-039 discharged: real bridge streams candles observed" (plus the date) in STATE.md discharges it; blocks phase-07-exit. The checkpoint was approved without the observation attached, so the row stays `waiting` in the owner's hands (R-11: an owner-judgment record is not self-discharged). The original check pattern matched STATE.md's own DUE NOW line (L-024 cause, third hit) and falsely reported done; repaired to the discharge-record anchor, red on today's file.
- **DEBT-040** (new, phase-07 reflect backlog): gate_all.py's responder identity — refuse a server whose /api/health does not match the expected fresh-ephemeral identity, stamp port + build into receipts. Origin: gate run 1 graded a stray dev hub on 8770, saved only by a loud timeout (L-029, subject axis).
- **DEBT-041** (new, phase-07 reflect backlog): debt.py creation-time check validation — negative-control every new check against today's target, reject DUE NOW-colliding patterns, sweep waiting/open rows. Origin: L-024 failed to bind at check-creation three times; DEBT-038's check swept clean today.

## Issues
- The kit's `design_lint.py check` is file://-only (`collect()` resolves `Path.as_uri()`); its served half is gate_all's design_lint receipt (green). The file:// run reports its own fetch noise (L-020).
- The agent could not visually inspect the screenshots: this session's model has no image input (verified at the phase reflect — the Read of a captured PNG returns "Media omitted … does not support image input"). The look pass rests on the gate receipts, the DOM assertions and the owner's look at the checkpoint (polish-log); L-050 records that the shots exist and a vision-capable session should Read them first.

## Reflect and RSI

- reflect: skill reflect (pstack panel: 3 reviewers + synthesizer over the four phase SUMMARYs; digest `.paul/runs/reflect-07-digest.md`) — 15 raw findings, **8 accepted, 1 rejected, 2 backlog**: L-024 promoted to validated at hits 3 (discharge-record anchor rule, DEBT-039's false green), L-027 generalized (reflect-the-secret proof, hits 2), L-029 promoted to validated at hits 3 (gate runs probe responder identity), L-048/L-049/L-050 added (unread-body deadlock, method-aware ctx-scoped route mocks, Read-the-shots look pass), D-034 (parallel_agents: false in this repo), R-12 (automated falsifiers before human-judgment gates; push-deferred phases record CI-unproven as an open exit condition), backlog → DEBT-040 (gate identity probe + receipt stamp) and DEBT-041 (debt.py creation-time check validation). Rejected: the auth-off loopback SSRF gating-parity finding — already covered by D-001 (auth-off binds 127.0.0.1 only), D-033 (the only loopback bridge is the operator's env-configured `local`) and the absence of any anonymous bridge surface (no database in local mode).
- instruction edits proposed: tune `design-taste-frontend`'s look-pass step ("Read the shots from `.paul/runs/shots-<plan>/`; state explicitly when the harness cannot render images") — held for the owner's explicit approval per the reflect skill's apply gate (a skill-body edit waits for the user's yes).
- rsi: no candidate — the three new candidate rows' evidence is already a reproduced failure with the fix verified by the gates (L-048: all browser gates timed out, then green after `r.text()` + `/auth/me`-first; L-049: driver cases 5/8 red until method-aware mocks, green after; L-050: the reflect session's own Read returned the media-omitted error verbatim); a bounded experiment would re-run a recorded failure, and L-031's reviewer-confirmation precedent marks the promotion line.
- ledger: added hub/settings.html (appearance ledger, huashu row).
- DEBT arithmetic at close: overdue 0, waiting 15 (13 + DEBT-040/041), done 26.

## Security review (transition gate, /security-review --full, whole phase-07 diff, 2026-11-26)

**No Critical, no High.** Two Medium, one Low, filed as DEBT-042 (agent, phase-08 trigger) with the
exact fixes named; the verified-good record:
- SSRF closed end to end: all four dial sites (register mt5.rs:96, test mt5.rs:208, REST bars
  market.rs:760, ws ws.rs:1071) vet with auth-on rules before any dial; Vetted.addrs pins the
  connect (DNS rebinding dead); redirects Policy::none + explicit refusal; numeric hosts refused
  pre-DNS; the auth-off http exception requires every resolved address loopback (guard.rs:177-179);
  bridge=local 404s whenever auth is on.
- Token secrecy: HMAC-SHA256 domain-separated key, XChaCha20-Poly1305 per-record nonces, opaque
  errors, no token in any response or error string; the Python bridge: constant-time
  compare_digest, refuses to start without BRIDGE_TOKEN, no access logs, loopback default bind.
- Cross-user: all five store statements scope user_id; foreign id 404; signed-out 401 before the id
  parses (test-pinned); anonymous symbols body byte-identical.
- CSRF/exposure: /api/me/mt5* in Area::Me (Origin check, JSON content-type, 300 KB cap); cookie
  HttpOnly; Secure; SameSite=Lax.
- Poller: refcounted per (bridge, symbol, tf), last unsubscribe aborts (no ws-close leak),
  Semaphore(2), 10 s/25 s budgets, 5 MB streamed cap (no gzip feature, no decompression bomb).
- settings.js: token is a password field, cleared from the DOM on every exit path, never in URL/
  console/history; server text rendered escaped.

Findings (dispositions — defer, do not reopen the applied tree):
1. **Medium — logbus redaction is per-process; a restart re-opens the 07-01 hole.** add_secret runs
   only at registration (routes/mt5.rs:129) and successful test (mt5.rs:250); boot re-arms only the
   local bridge (lib.rs:411-419). After a hub restart, named-bridge tokens are absent from the
   redaction set until a re-test, so a hostile bridge echoing the Authorization value lands it
   verbatim in the ring (client.rs:300-308 → feeds/bridge.rs:183). Fix: in hosted AppState::start,
   after Db::open, add_secret every opened token + URL query; same for the test-failure branch.
2. **Medium — unbounded per-user resources.** No cap on registrations per user, and BridgeClient's
   pinned-client cache never evicts (client.rs:89, :138); a scripted user with a DNS-rotating host
   grows both unboundedly. Fix: cap bridges per user at insert; bound or evict the client cache.
3. **Low — guard misses adjacent v4/v6 classes.** `::/96` IPv4-compatible (not caught by
   to_ipv4_mapped), `0.0.0.0/8` non-exact, `240.0.0.0/4`, `198.18.0.0/15` pass refused_v6/v4
   (guard.rs:98-121). Modern stacks do not route these to loopback; a stated-class completeness fix.
Phase 07 is closed 2026-11-26: 4 of 4 plans unified, the reflect + rsi run, /security-review --full clean (findings filed as DEBT-042), and DEBT-039 discharged (the owner's real-bridge observation, gate phase-07-exit exit 0). What remains: the phase commit (wave-1 + 07-03 + 07-04 together, auto_commit: false — the owner asks first; the first push after it discharges DEBT-038).
