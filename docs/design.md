# Design

Like opening a repository on GitHub or GitLab, minus the social layer: same places, same
words, calmer surface.

Colours, type, space and motion come only from `theme/schwarzwald/`, a vendored snapshot that
is replaced as a whole, never edited by hand; its README says how to update it. gitterrace
reads the tokens and solves none. `scripts/theme-gen.py` writes the block in
`crates/gitterrace-web/static/tokens.css` and the Rust constants `gitterrace-cards` uses.
`make theme` verifies both.

- No raw colour outside `tokens.css` (a test enforces it). Components use the role aliases,
  never the `--n-*` ramp. A new token comes with a new snapshot, never a hand edit.
- Charts are server-rendered SVG coloured by class, never inline.

<!-- BEGIN GENERATED: tokens -->
The role aliases in `tokens.css`: `--bg`, `--surface`, `--soft`, `--hover`, `--line`, `--line-strong`, `--accent-ink`, `--faint`, `--muted`, `--fg`, `--serif`, `--sans`, `--mono`, `--page-pad`.

<!-- END GENERATED: tokens -->
