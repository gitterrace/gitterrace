# gitterrace design

gitterrace should feel like opening a repository on GitHub or GitLab, minus the
social layer: same places for the same things, same words, calmer surface.
This file is the contract for every page, chart and shareable card.

Tokens live in `crates/gitterrace-web/static/tokens.css`. The palette is mirrored
in `crates/gitterrace-web/src/palette.rs` for server-rendered SVG and
`gitterrace-cards`; a unit test fails if the two drift.

## Principles

1. **Content first.** Code, history and numbers are the interface. Chrome is
   hairlines and whitespace, never gradients, glows or decorative color.
2. **One accent.** Teal marks what is interactive or current: links, focus,
   the active tab, the primary series. If everything is teal, nothing is.
3. **Color means something.** Green and red are only additions and
   deletions. Categorical colors only distinguish series. Never use a
   semantic color for decoration.
4. **Familiar placement.** Repo name and branch top left, tabs under it
   (Code, Commits, Branches & Tags, Insights), last commit bar above the file
   table, About in the right column. Use GitHub's words.
5. **Both themes are first class.** Every token has a light and a dark value.
   Check every change in both, and at 390px wide.
6. **Quiet motion.** Only hover/focus color changes and progress. Respect
   `prefers-reduced-motion` (the duration tokens drop to 0).

## Tokens

### Color roles

Components use roles, not the raw ramp. The neutral ramp `--n-0..9` exists so
roles can be retuned in one place.

| Role            | Use                                         |
| --------------- | ------------------------------------------- |
| `--bg`          | page background                             |
| `--surface`     | cards, header, inputs                       |
| `--soft`        | table heads, file headers, code blocks      |
| `--hover`       | row and button hover                        |
| `--line`        | every border and divider                    |
| `--line-strong` | hovered borders, input borders              |
| `--faint`       | line numbers, axis ticks, placeholders      |
| `--muted`       | secondary text, metadata                    |
| `--fg`          | primary text                                |
| `--accent`      | interactive and current                     |
| `--accent-fg`   | link text (darker/lighter for contrast)     |
| `--accent-soft` | selected row, active filter background      |
| `--ring`        | focus outline and text selection            |

Text contrast: `--fg` and `--muted` pass WCAG AA on `--bg` and `--surface` in
both themes; `--faint` is for non-essential text only.

### Semantic

`--add/-bg/-hl`, `--del/-bg/-hl` for diffs and line counts. `--warn`,
`--info`, `--done` (merged, important) for alerts and badges, each with a
`-bg` where a tinted fill is needed.

### Data

- **Sequential** `--seq-0..4`: heatmaps and intensity. `seq-0` is "no data"
  and sits just off the background so the grid stays readable. Single-hue
  teal, monotonic in lightness, so it reads in grayscale and for every
  color-vision type.
- **Categorical** `--cat-1..8`: languages, commit types, authors. Order is
  Okabe-Ito (blue, orange, green, pink, sky, vermillion, yellow), the
  standard colorblind-safe set, retuned for dark backgrounds. `--cat-8` is
  always "other" and is neutral. Assign in order of size; never more than 7
  named series, fold the rest into other. Always pair color with a legend or
  a label.

### Type

System sans and system mono, no web fonts (offline, instant).

| Token         | px  | Use                                  |
| ------------- | --- | ------------------------------------ |
| `--text-xs`   | 12  | badges, legends, axis labels         |
| `--text-sm`   | 13  | metadata, code, tables of numbers    |
| `--text-base` | 14  | body UI text (GitHub's density)      |
| `--text-md`   | 16  | rendered markdown body, repo name    |
| `--text-lg`   | 20  | page titles, breadcrumbs             |
| `--text-xl`   | 24  | stat numbers                         |
| `--text-2xl`  | 32  | hero numbers on cards                |

Weights: 400, 500, 600 only. Section labels are sentence case at 14/600, not
spaced uppercase.

### Space, shape, depth, motion

- Space on a 4px grid: `--space-1` (4) to `--space-16` (64). Card padding is
  16, section gap 32, page top 24.
- Radii: `--radius-sm` 4 (inline code, chips), `--radius` 6 (buttons, inputs,
  cards), `--radius-lg` 10 (shareable cards only), `--radius-full` pills.
- Borders do the work; `--shadow` is a 1px lift, `--shadow-2` only for
  popovers (finder, branch switcher).
- Motion: `--dur-fast` 120ms for hover, `--dur` 200ms for popovers,
  `--dur-slow` 400ms for progress, easing `--ease`.

## Components

- **Header.** Surface background, logo mark, repo name (16/600), branch pill
  in mono. Tabs sit on the header's bottom border; the active tab gets a 2px
  accent underline and `--fg` text, others `--muted`. No filled tab
  backgrounds.
- **Tabs (in page).** Same underline pattern as the header, on a `--line`
  rule.
- **Cards / boxes.** `--surface`, `--border`, `--radius`, no padding on the
  box itself when it holds rows. A box header row uses `--soft`.
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

Three stacked rounded gitterrace, top one in accent, the lower two in neutral
steps. It is drawn with `currentColor`-free classes so it follows the theme.
Favicon is the same mark.

## Do / don't

- Do use a role token; don't reach for `--n-*` or a hex value.
- Do add a token (both themes, plus `palette.rs` if charts need it) rather
  than inline a one-off color.
- Do keep text at 14px in UI; don't shrink metadata below 12px.
- Don't use uppercase letter-spaced labels; GitHub moved away from them, and
  they read as shouting next to calm content.
- Don't add shadows to rows or make cards float; borders are enough.
- Don't use green/red outside add/delete and status.
