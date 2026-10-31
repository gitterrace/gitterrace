# gitterrace design

gitterrace should feel like opening a repository on GitHub or GitLab, minus
the social layer: same places for the same things, same words, calmer
surface. Its voice is a survey sheet: a serif title, a quiet grotesk, mono
for everything a machine wrote, hairlines instead of boxes. A sibling of
tendir, a cousin of fly.io's docs. This file is the contract for every page,
chart and shareable card.

Tokens live in `crates/gitterrace-web/static/tokens.css`. The palette is mirrored
in `crates/gitterrace-web/src/palette.rs` for server-rendered SVG and
`gitterrace-cards`; a unit test fails if the two drift, and another fails if any
other stylesheet holds a raw colour.

## Principles

1. **Content first.** Code, history and numbers are the interface. Chrome is
   hairlines and whitespace, never gradients, glows or decorative colour.
2. **Ink, not colour.** Text is near-neutral (OKLCH chroma 0.016, hue 250):
   the theme is felt, never seen. Hierarchy comes from one lightness ladder,
   not from tint.
3. **Colour is spent where it means something.** The accent marks what you
   act on, pigments mark kinds (languages, commit types, authors), signals
   mark state (added, removed, warning). Each has its own lightness and
   chroma band.
4. **Three voices, each with one job.** Fraunces for titles, Bricolage
   Grotesque for prose and names, the system mono for anything a machine
   wrote: shas, paths, counts, dates, labels.
5. **Familiar placement.** Repo name and branch top left, tabs under it
   (Code, Commits, Branches & Tags, Insights), last commit bar above the file
   table, About in the right column. Use GitHub's words.
6. **Both themes are first class.** Every token has a light and a dark value.
   Check every change in both, and at 390px wide.
7. **Every value is solved, not picked.** Each colour has a target OKLCH and
   a reason; contrast is measured, not eyeballed.
8. **Quiet motion.** Only hover/focus colour changes and progress. Respect
   `prefers-reduced-motion`.

## Not the AI look

What makes a UI read as generated, and what gitterrace does instead:

| Avoid                                           | Instead                                    |
| ----------------------------------------------- | ------------------------------------------ |
| Rounded cards with soft drop shadows everywhere | Hairline rules, 3px corners, no shadows    |
| Teal/purple gradient accents and glows          | One flat accent, spent only on actions     |
| Pill chips and pill buttons                     | Square-ish tags in mono                    |
| One sans at 14px for everything                 | Serif titles, grotesk prose, mono metadata |
| Centered hero spacing, huge even padding        | Left-aligned, dense where data is dense    |
| Boxes inside boxes                              | One frame per list, sections are rules     |
| Decorative outline icon sets                    | Icons only for file kinds                  |
| Low-contrast grey-on-grey text                  | A measured ink ladder, AA or better        |
| Glassy blurred sticky headers                   | Plain page-coloured header                 |
| Donut charts and decorative sparklines          | Bars, heatmaps, tables with numbers        |
| Illustrated empty states                        | One muted sentence                         |

## Colour: Kandel, from tendir

The colours are tendir's Kandel theme (indigo dusk, hue 280), taken from
`tendir/docs/generated/theme-tokens.json`: ink, accent, signals and pigments
are its exact values. Every colour Kandel does not define (surfaces, header,
diff rows and words, selection, heat ramp, syntax) is solved from Kandel's
own tokens by `scripts/solve-theme.py`, which writes the colour block of
`tokens.css` and checks every floor; `--check` fails on drift. The rules, and
the same roles solved for Elz, Hochburg and Schauinsland, are in
[theme-extension.md](theme-extension.md), the proposal back to tendir. Only
Kandel ships: a theme picker needs template and script changes, so the other
three live in docs until tendir adopts the roles.

- **Ink ladder.** Light primary / secondary / tertiary L 0.270 / 0.432 /
  0.569, dark 0.939 / 0.816 / 0.734, all C 0.016 H 250. Contrast on `--bg`:
  light 14.4 / 7.7 / 4.3, dark 15.6 / 10.5 / 7.9.
- **Accent.** Two tokens. `--accent` (tendir's accentInk) is the mark:
  links, focus, the active series, the logo's HEAD tread; 5.9:1 light, 7.5:1
  dark. `--accent-fill` (tendir's accent) is a fill under a white label;
  9.3:1 light, 5.3:1 dark.
- **Signals.** Additions and "good" are Tanneschiefer, deletions and errors
  are tendir's overdue, warning is tendir's warning; info is the secondary
  ink. Each clears 4.5:1 as text in both themes. Add vs delete OKLab
  separation (x100): normal 22.8 / 25.1, deuteranopia 8.9 / 8.5, tritanopia
  28.1 / 31.8, protanopia 6.0 / 15.7 (light / dark); worst case in
  CIEDE2000 7.5 / 15.5, which meets tendir's own signal minimum (7.4). The row tints are the signals at 10 to
  14% and keep only 0.6 / 2.6 under CVD; widening them would break tendir's
  fill rules, so a diff never relies on colour alone: rows carry `+`/`-`
  glyphs and counts carry their sign.
- **Sequential.** `--seq-0` is the hover surface; `--seq-1..4` take equal
  OKLCH lightness steps to the accent ink at hue 280, so a heatmap reads in
  grayscale.
- **Categorical.** tendir's entity pigments in this order: Blue, Orange,
  Green, Pink, Teal, Red, Yellow; `--cat-8` is "other" and neutral. One
  lightness band per theme (L about 0.5 light, 0.7 dark), so no series
  shouts. Never more than 7 named series; always pair with a label.

### Token table

Generated from `scripts/solve-theme.py`; L C H are OKLCH (hue "-" means neutral).

| Token | Role | Light hex | Light L C H | Dark hex | Dark L C H |
| --- | --- | --- | --- | --- | --- |
| `--n-0` | surface | `#ffffff` | 1.000 0.000 - | `#191a1a` | 0.217 0.002 - |
| `--n-1` | bg | `#fbfbfb` | 0.988 0.000 - | `#131313` | 0.187 0.000 - |
| `--n-2` | soft, code | `#f6f6f7` | 0.973 0.001 - | `#1d1e1e` | 0.234 0.002 - |
| `--n-3` | hover | `#eff0f0` | 0.954 0.001 - | `#242425` | 0.261 0.002 - |
| `--n-4` | line, grid | `#e2e3e4` | 0.915 0.002 - | `#2c2d2e` | 0.297 0.002 - |
| `--n-5` | line-strong, axis | `#c8c9cb` | 0.836 0.003 - | `#414345` | 0.382 0.004 - |
| `--n-6` | faint (ink.tertiary) | `#707881` | 0.569 0.017 251 | `#a1aab3` | 0.734 0.017 248 |
| `--n-7` | muted (ink.secondary) | `#4a5159` | 0.432 0.016 252 | `#bbc4cd` | 0.816 0.016 248 |
| `--n-9` | fg (ink.primary) | `#21272e` | 0.270 0.016 252 | `#e3ecf6` | 0.939 0.017 251 |
| `--header` | header band | `#edf0f4` | 0.954 0.006 - | `#090d11` | 0.157 0.011 248 |
| `--raised` | popover, dialog | `#ffffff` | 1.000 0.000 - | `#242425` | 0.261 0.002 - |
| `--scrim` | overlay (black 30%) | `#0000004d` | alpha | `#0000004d` | alpha |
| `--accent` | accent ink (accentInk) | `#5b5e8d` | 0.499 0.075 281 | `#9ca1d4` | 0.723 0.074 280 |
| `--accent-fill` | accent fill | `#414468` | 0.400 0.061 280 | `#65698f` | 0.532 0.060 279 |
| `--accent-soft` | selected row | `#eceff3` | 0.951 0.006 - | `#262c31` | 0.289 0.012 243 |
| `--selection` | text selection | `#d1d2db` | 0.866 0.012 281 | `#343643` | 0.336 0.023 278 |
| `--focus` | focus ring | `#5b5e8d` | 0.499 0.075 281 | `#9ca1d4` | 0.723 0.074 280 |
| `--add` | add (Tanneschiefer) | `#236958` | 0.472 0.075 174 | `#78c9b7` | 0.779 0.084 178 |
| `--add-bg` | add row | `#e9f0ee` | 0.949 0.008 - | `#263330` | 0.308 0.018 179 |
| `--add-hl` | add word | `#cadbd7` | 0.877 0.019 180 | `#354e49` | 0.402 0.032 181 |
| `--del` | del (overdue) | `#af383d` | 0.516 0.155 22 | `#ff7778` | 0.730 0.166 22 |
| `--del-bg` | del row | `#f7ebec` | 0.950 0.013 12 | `#392727` | 0.294 0.027 19 |
| `--del-hl` | del word | `#eccfd0` | 0.879 0.033 15 | `#5e3636` | 0.382 0.058 20 |
| `--warn` | warn | `#875900` | 0.502 0.106 74 | `#e1a447` | 0.760 0.130 75 |
| `--warn-bg` | warn row | `#f3eee6` | 0.951 0.012 80 | `#352d20` | 0.302 0.025 80 |
| `--info` | info (deferred) | `#57646d` | 0.495 0.022 238 | `#9fadb6` | 0.739 0.020 236 |
| `--seq-0` | heat 0 | `#eff0f0` | 0.954 0.001 - | `#242425` | 0.261 0.002 - |
| `--seq-1` | heat 1 | `#c8cad8` | 0.842 0.019 279 | `#3f404c` | 0.375 0.020 282 |
| `--seq-2` | heat 2 | `#a2a5bf` | 0.728 0.038 280 | `#5c5f77` | 0.492 0.038 279 |
| `--seq-3` | heat 3 | `#7d81a6` | 0.614 0.056 280 | `#7b7fa4` | 0.607 0.056 280 |
| `--seq-4` | heat 4 | `#5b5e8d` | 0.499 0.075 281 | `#9ca1d4` | 0.723 0.074 280 |
| `--cat-1` | series 1 | `#306693` | 0.495 0.092 246 | `#66a6dd` | 0.704 0.105 246 |
| `--cat-2` | series 2 | `#8c5528` | 0.503 0.094 57 | `#cf945f` | 0.713 0.099 62 |
| `--cat-3` | series 3 | `#127050` | 0.485 0.098 164 | `#50b291` | 0.695 0.105 168 |
| `--cat-4` | series 4 | `#6e3a52` | 0.421 0.080 352 | `#d28caf` | 0.721 0.096 348 |
| `--cat-5` | series 5 | `#1f6a72` | 0.483 0.072 205 | `#4caeb8` | 0.697 0.092 204 |
| `--cat-6` | series 6 | `#994b4c` | 0.510 0.105 21 | `#de898e` | 0.720 0.104 16 |
| `--cat-7` | series 7 | `#706226` | 0.497 0.081 96 | `#aea25c` | 0.707 0.092 100 |
| `--cat-8` | series 8 | `#c8c9cb` | 0.836 0.003 - | `#414345` | 0.382 0.004 - |
| `--syn-comment` | comment | `#6a727b` | 0.548 0.017 251 | `#a1aab3` | 0.734 0.017 248 |
| `--syn-keyword` | keyword | `#513761` | 0.385 0.076 312 | `#b992d2` | 0.719 0.100 312 |
| `--syn-string` | string | `#127050` | 0.485 0.098 164 | `#50b291` | 0.695 0.105 168 |
| `--syn-constant` | constant | `#306693` | 0.495 0.092 246 | `#66a6dd` | 0.704 0.105 246 |
| `--syn-entity` | entity | `#5b5e8d` | 0.499 0.075 281 | `#9ca1d4` | 0.723 0.074 280 |
| `--syn-tag` | tag | `#1f6a72` | 0.483 0.072 205 | `#4caeb8` | 0.697 0.092 204 |
| `--syn-param` | param | `#8c5528` | 0.503 0.094 57 | `#cf945f` | 0.713 0.099 62 |

### Type

Two vendored OFL families, Latin subsets, about 68KB woff2 in total, served
from the binary (no CDN): **Fraunces** 600 for titles (a warm, slightly
quirky serif standing in for fly.io's commercial Mackinac) and **Bricolage
Grotesque** 400/600 for body. Mono is the system's (`ui-monospace`, SF Mono,
Menlo), which every developer machine already renders well.

| Token         | px  | Use                                       |
| ------------- | --- | ----------------------------------------- |
| `--text-xs`   | 12  | legends, axis labels                      |
| `--text-sm`   | 13  | metadata, code, tables of numbers         |
| `--text-base` | 15  | body UI text                              |
| `--text-md`   | 16  | rendered markdown body                    |
| `--text-lg`   | 20  | small titles                              |
| -             | 26  | page titles and breadcrumbs (Fraunces)    |
| -             | 11  | section labels and tabs (mono, uppercase) |

Weights: 400 and 600.

## Adding a page

New pages (insights, export) extend `base.html`, put their sub-navigation in
`tabs.html`, use `<h2>` for section labels (the rule draws itself) and
`.card` for a framed list or chart. Charts come from `charts.rs` and colour
marks by class (`.bar`, `.c1..c8`, `.h0..h4`); never inline a colour.

### Space, shape, depth, motion

- Space on a 4px grid: `--space-1` (4) to `--space-16` (64). Card padding is
  16, section gap 32, page top 24.
- Radii are nearly square: `--radius-sm` 2, `--radius` 3, `--radius-lg` 4.
  `--radius-full` is 3 as well: there are no pills.
- Hairlines do the work. No shadows on the page; `--shadow-2` only for
  popovers (finder, branch switcher).
- Motion: `--dur-fast` 120ms for hover, `--dur` 200ms for popovers,
  `--dur-slow` 400ms for progress, easing `--ease`.

## Components

- **Header.** Page background, the mark, `gitterrace / repo` in Fraunces,
  branch switcher in mono. Tabs are mono small caps on the header's bottom
  rule; the active tab gets a 1px `--fg` underline. No filled tab
  backgrounds.
- **Tabs (in page).** Same underline pattern as the header, on a `--line`
  rule.
- **Section labels.** Mono 11px uppercase, tracked .12em, `--muted`, followed
  by a hairline that runs to the edge (a survey sheet's legend line).
- **Boxes.** Page background, `--border`, `--radius`. A box header row uses
  `--soft`. Boxes only where rows need a frame; never nest them.
- **Tables and lists.** Rows 40px tall (8px block padding at 14px text),
  divided by `--line`, hover `--hover`. Numbers right aligned with
  tabular figures. Names in `--fg`, metadata in `--muted`.
- **Diff.** Mono 13px with 20px lines. Row background `--add-bg`/`--del-bg`,
  gutter `--add-hl`/`--del-hl`, hunk header `--soft` in `--muted`. Never
  color the code text itself.
- **Code / blob.** Line numbers `--faint`, target line `--accent-soft` with
  a 2px accent inset. Syntax colors come from `--syn-*` only.
- **Charts.** Server-rendered SVG, colored through classes mapped to tokens
  (so themes switch without re-render). Gridlines `--line`, ticks `--faint`
  12px. Single-series bars use `--accent`; stacked use `--cat-*`; heatmaps
  use `--seq-*`. Every chart has a `<title>` per mark and an `aria-label`.
- **Empty states.** One muted sentence that says what would be here and,
  where useful, how to get it. No illustrations.
- **Loading.** Indexing shows a thin accent progress bar and the count; page
  chrome renders immediately.
- **Badges and shareable cards** (`gitterrace-cards`). Use `palette.rs`, not
  hand-picked hex. Badges: label on `--n-8`-ish neutral, value on accent,
  12px, radius 4. Cards: `--radius-lg`, surface, 24px padding, the logo mark
  bottom left, one number large (32/600) with a muted caption, at most one
  chart in the card's palette. Render both themes.

## Logo

The gitterrace mark is a lowercase t whose stem steps down in terraces; the
last tread is HEAD. The stem is `--fg`, the tread `--accent-ink`, so it follows
the theme. Sources and wordmarks live in `assets/brand/`; the header inlines
the same paths. Favicon is the 16px-snapped variant.

## Do / don't

- Do use a role token; don't reach for `--n-*` or a hex value.
- Do add a token (both themes, plus `palette.rs` if charts need it) rather
  than inline a one-off color, and give it a target L/C/H in the table above.
- Do keep body text at 15px; don't shrink metadata below 11px.
- Don't add shadows to rows or make boxes float; hairlines are enough.
- Don't use green or red outside add/delete and status.
- Don't add a third family; the mono is the system's.
