# Design track state

## vtradingview  (spec: .paul/design/specs/vtradingview.md)

| Step | Owner | Artifact (done when it exists) | Status | Note |
|---|---|---|---|---|
| D1 research | frontend-reference-hunt | .paul/design/reference-board-vtradingview.md | skipped | existing product: the reference is the current hub UI (hub/hub.html, hub/theme.js); user asked to design on the current hub theme (2026-11-23) |
| D2 extract | taste | .paul/design/taste/hub-theme.md (1-2 refs of the chosen direction) | done | extracted from hub/theme.js + hub.html (own product, palette adopted by user instruction); contrast measured, 4 fixes listed for D3 |
| D3 draft | egm-draft (+ design-taste-frontend draw.mjs --options 3; huashu-design renders the 3 dealt directions) | .paul/design/specs/vtradingview.md with direction id + seed, and .paul/design/directions/vtradingview/direction-approved.md | done | exception: direction hub-native fixed by owner, seed none (no draw.mjs deal); spec .paul/design/specs/vtradingview.md + direction-approved.md exist |
| D4 mockup (optional) | open-design-lifecycle (fallback: huashu-design when OpenDesign is unavailable) + huashu-design lint | .paul/design/opendesign/vtradingview/ and a D4 entry in .paul/design/polish-log.md | done | 7 screens built with huashu-design (OpenDesign not connected); design_lint red 0 on all 7; type floor in shared.css; polish-log.md D4 entry |
| D5 review | egm-review feature_spec | .paul/design/reviews/vtradingview/critic_report.md | done | feature_spec review stages 5-8 ran; owner concluded with findings open: Blockers C1 CSRF, C2 Risks; ~30 Majors; E1 self-hosting unanswered; reports in .paul/design/reviews/vtradingview/ |
| D6 readiness | egm-review implementation | .paul/design/reviews/vtradingview/implementation_readiness_report.md | done | implementation review ran; owner concluded with findings open: 7 Blockers (R1-R7) and 26 Majors in the plan, plus Stage 7 C1/C2, Stage 8 N1-N5, E1; see implementation_readiness_report.md |
