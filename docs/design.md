# gitterrace design

gitterrace should feel like opening a repository on GitHub or GitLab, minus
the social layer: same places for the same things, same words, calmer surface.
The principles, colours, type, space, radius, shadow and motion are the shared
schwarzwald contract. This file holds only what is specific to gitterrace.

- Shared contract: `schwarzwald/contract/shared.md`.
- gitterrace extension (code, heat, series roles):
  `schwarzwald/contract/extension-gitterrace.md`. See also
  [theme-extension.md](theme-extension.md).

## How the theme gets here

`theme/schwarzwald/` is a committed snapshot, written by schwarzwald's
`sync.py` and never edited by hand. `scripts/check-theme-snapshot.py` compares
each file's sha256 with `manifest.json`. `scripts/gen-theme.py` turns the
snapshot into what the code reads:

- `crates/gitterrace-web/static/tokens.css`, between its markers: Kandel light and
  dark, fonts, type scale, space grid, radius, motion.
- `theme/schwarzwald/rust`: the vendored `schwarzwald` crate (no dependencies),
  the same colours as Rust constants. `gitterrace-web` re-exports it from
  `palette.rs`; `gitterrace-cards` reads `KANDEL` directly.

`make theme` runs both checks and is part of `make check` and CI. To update:
run `sync.py`, then `scripts/gen-theme.py`, then commit. Only Kandel ships.
Another theme needs template and script changes for a picker.

Fonts are served from the snapshot (`theme/schwarzwald/fonts/`) by the font
route, so there is one copy. A stylesheet other than `tokens.css` holds no raw
colour; a test in `crates/gitterrace-web/tests/theme.rs` enforces it.

## Token names

Components use role aliases defined under the markers in `tokens.css`
(`--bg`, `--surface`, `--soft`, `--hover`, `--line`, `--faint`, `--muted`,
`--fg`, `--accent`, `--add`, `--del`, `--seq-*`, `--cat-*`, `--syn-*`), never
the `--n-*` ramp. Type steps are `--text-2xs` (11) to `--text-xl` (26). Fonts
are `--serif`, `--sans`, `--mono`, aliases of the shared `--font-title`,
`--font-body`, `--font-mono`. Add a token in schwarzwald, not here.

## Page anatomy

New pages extend `base.html`, put their sub-navigation in `tabs.html`, use
`<h2>` for section labels (the rule draws itself) and `.card` for a framed list
or chart. Charts come from `charts.rs` and colour marks by class (`.bar`,
`.c1..c8`, `.h0..h4`); never inline a colour.

- **Header.** The mark, `gitterrace / repo` in the serif, branch switcher in
  mono. Tabs on the header's bottom rule, the active one underlined.
- **Layout.** Repo name and branch top left, tabs under it (Code, Commits,
  Branches & Tags, Insights), last commit bar above the file table, About in
  the right column. Use GitHub's words.
- **Diff and blob.** As the gitterrace extension: mono 13px, 20px lines, row
  tints, `+` and `-` glyphs, target line with a 2px accent inset.
- **Charts.** Server-rendered SVG coloured through classes. Single series
  `--accent`, stacked `--cat-*`, heatmaps `--seq-*`. A `<title>` per mark and
  an `aria-label`.
- **Badges and cards** (`gitterrace-cards`). Colours from the `schwarzwald` crate.

## Logo

The mark is a lowercase t whose stem steps down in terraces; the last tread is
HEAD. The stem is `--fg`, the tread `--accent`, so it follows the theme.
Sources and wordmarks live in `assets/brand/`.
