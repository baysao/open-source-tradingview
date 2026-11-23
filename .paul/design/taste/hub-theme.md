# Taste extract: TradeX hub (own product)

Source: hub/theme.js and hub/hub.html in this repo, read 2026-11-23. Not a crawled competitor.
Deviation from the D2 brief, by user instruction ("based on current theme from hub"): this file DOES
adopt palette and type, because the new screens must look native to the existing hub. The D5
competitor test does not apply to the palette; it still applies to structure.

## Palette (CSS variables set by HubThemeApply on :root)

Keys: --bg, --panel, --raised, --line, --grid, --text, --dim, --faint, --accent, --accentInk, --up,
--down, --ok, --bad, plus --font and --mono. Presets: graphite (default, dark), black (dark), light.
Users can override any key in Settings, so new screens must use only these variables, never hex.

| Role | graphite | black | light |
|---|---|---|---|
| bg | #151412 | #000000 | #ffffff |
| panel | #1b1a17 | #0b0b0b | #f6f4f0 |
| raised | #24221e | #181818 | #ebe7e0 |
| line | #2f2c27 | #262626 | #dcd6cc |
| text | #e6dfd2 | #ececec | #1d1b18 |
| dim | #958c7e | #8a8a8a | #6f685e |
| faint | #5e584f | #4a4a4a | #b3ab9f |
| accent / accentInk | #e3a33a / #1a1407 | #e3a33a / #141005 | #b8761a / #ffffff |
| ok / bad | #8fb36a / #c9553c | #8fb36a / #c9553c | #4e7d2c / #c0442b |

## Measured contrast (WCAG ratio, computed in this session from the hex values above)

| Pair | graphite | black | light | Verdict for new screens |
|---|---|---|---|---|
| text on panel | 13.13 | 16.66 | 15.64 | body text: pass |
| dim on panel | 5.25 | 5.70 | 5.01 | secondary text: pass |
| dim on raised | 4.79 | 5.14 | 4.46 | light fails 4.5: no dim text on raised in light |
| faint on panel | 2.47 | 2.22 | 2.07 | fails: decoration only, never text or placeholder |
| accent on panel | 7.93 | 8.97 | 3.39 | light: large text or icons only |
| accentInk on accent | 8.34 | 8.65 | 3.72 | light primary button label fails 4.5 |
| ok on panel | 7.31 | 8.27 | 4.45 | light fails 4.5 by a hair |
| bad on panel | 4.02 | 4.54 | 4.66 | graphite error text fails 4.5 |
| line on panel | 1.25 | 1.30 | 1.32 | fails 3:1: cannot be the only input boundary |

Consequences the spec must resolve (D3), not this file:
1. Light preset: primary button needs a darker accent or dark ink (accentInk on accent is 3.72).
2. Error message text (MT5 test failure, sign-in failure): graphite `bad` is 4.02; use text color with a
   `bad` icon, or a lighter error tone that clears 4.5.
3. Input fields (MT5 URL, token) need a boundary that reaches 3:1 against panel; `line` is about 1.3.
4. Placeholder text must use `dim`, not `faint`.

## Type

- UI font: --font, default "Segoe UI Variable Text", "Segoe UI", system-ui; user can switch to Nunito,
  Manrope, Quicksand or Inter (Google Fonts). New screens must render in all five.
- Numbers, tokens, URLs: --mono ("Cascadia Mono", Consolas, monospace).
- Sizes in hub.html: 11px and 12px dominate (toolbar, menus), 13px and 14px for body, 18px and 24px for
  the few headings. Dense trading-terminal scale: keep it, but body copy in dialogs stays at 13px or more.

## Structure, density, restraint

- Flat surfaces in three steps (bg, panel, raised) separated by 1px `line` hairlines; no shadows as the
  main separator, no gradients.
- Radii are small: 3px is the norm, 4 to 6px for panels, 8px once, 50% for round items. Avatar is round;
  dialogs at 6px.
- Accent amber is rare: active tool, selected item, the one primary action. One amber thing per view.
- Up color is amber, not green, in the dark presets: never use `up`/`down` for success/error states;
  use `ok`/`bad`.
- Menus and panels are compact (12px text, tight rows); the chart owns the screen. New UI lives in the
  header, a dropdown, a dialog or the existing Settings panel, never a new full page except privacy.

OPEN
- motion: the hub has no motion system; spec decides whether menus animate (respect reduced motion).
