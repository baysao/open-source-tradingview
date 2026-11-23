# vtradingview new screens: direction `hub-native`

Built with the huashu-design web module as the documented fallback (OpenDesign MCP not connected, no
BYOK). Direction fixed by the owner ("based on current theme from `hub`"), so no directions were dealt;
see `.paul/design/directions/vtradingview/direction-approved.md`. Source of the look: `hub/theme.js` and
`hub/hub.html` in this repo, summarised in `.paul/design/taste/hub-theme.md`.

## How the mockups load the theme

Every screen loads `../../../../hub/theme.js` (the real file, unmodified), then `shared.css` and
`shared.js`. Serve from the project root (`python -m http.server`) so the relative path resolves. The
preset switcher (Graphite, Black, Light; a button group, no native select) copies `HubPresets[name]` onto
`:root` without writing `localStorage`, so it never changes the user's real hub theme. `?preset=light`
opens a page in a given preset.

`shared.js` derives two colours at run time, from the user's actual values:

- `--onAccent`: whichever of `--accentInk` and `--text` contrasts more with `--accent` (graphite and
  black keep accentInk, light uses text).
- `--lift`: the colour opposite `--onAccent`; primary hover and active mix accent toward it, so the
  label never loses contrast.

## Rules applied

| Rule | How |
|---|---|
| Colours | Only theme.js variables plus `--onAccent` and `--lift`; no hex in component CSS |
| One amber per view | The single primary action per frame; the sign-in button is secondary because the bar already shows amber (brand, active timeframe) |
| Destructive | `.btn.danger`: full `--bad` border, `--bad` icon, a 16 percent `--bad` tint over panel, label in `--text`. Same style on trigger and confirm (delete account, delete bridge) |
| Error text | `--text` with a `--bad` icon and the word "Error:" |
| Inputs | Border `--dim`, focus `--accent`, error border `--bad`; placeholder and hint `--dim`; `--faint` never carries text |
| Light preset | No `--dim` text on `--raised`: hover rows switch secondary text to `--text` |
| Surfaces | bg, panel, raised with 1px `--line` hairlines; no shadow separators, no gradients, no one-sided borders |
| Radii and type | 3, 4, 6 px; UI text 11 to 13 px, dialog body 13 px, privacy body 14 px |
| States | default, hover, focus-visible ring (2px accent), active, disabled (opacity .5, no pointer events), busy (spinner plus aria-busy), error, pressed |
| Mobile first | Base styles at 320 px; 600 px and 960 px layers by min-width |
| Motion | Only the spinner and 120 ms colour transitions; both stop under reduced motion |
| Icons | Lucide paths in an injected sprite, `currentColor` |

## Screens

| File | States on the page |
|---|---|
| `01-signin-avatar-menu.html` | signed out, signing in, sign-in failed, signed in with menu, admin variant |
| `02-sync-notices.html` | saved, saving, offline with retry, conflict (load the other version / keep and overwrite), backups with Restore per key and a restored row |
| `03-mt5-bridges.html` | empty, list with inline delete confirm, add form, testing, connected with symbol count, error with the exact cause and the other three causes |
| `04-delete-account.html` | trigger, confirm, busy, error, plus a live modal (focus trap, Escape, focus return) |
| `05-admin-log-panel.html` | TTFC p75, per-feed health, restart confirm, restarting, live redacted log with follow toggle (sample data) |
| `06-desktop-signin-waiting.html` | waiting, failed with steps, after cancel |
| `07-privacy.html` | static page, marked as a draft for the owner to confirm |

## Tokens

`tokens.json` here is the hub theme in DTCG form (three presets with `onAccent`, fonts, sizes, radii,
space). `design-tokens.json` at the project root is the one-time 3-tier conversion: light preset in the
semantic tier, graphite in the dark tier, all three presets as primitives. `hub/theme.js` stays the
runtime source; the token files describe it.

## Open

- Google's own sign-in button (GIS) renders Google's branding; the mockup uses a theme-native stand-in.
- The hub brand word is amber on panel: 3.39 in the light preset per hub-theme.md, below 4.5 for 14 px
  bold text. Existing hub behaviour, not changed here.
- Sample figures (412 symbols, 1.42 s, lags) are illustrations, not measurements.
