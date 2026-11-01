# Design

Like opening a repository on GitHub or GitLab, minus the social layer: same places, same
words, calmer surface.

Colours, type, space and motion come from the shared schwarzwald contract (Kandel). gitterrace
reads them and solves none. `theme/schwarzwald/` is a lean snapshot written by schwarzwald's
`sync.py`; its README says how to update it. `scripts/gen-theme.py` writes the block in
`crates/gitterrace-web/static/tokens.css` and the Rust constants `gitterrace-cards` uses.
`make theme` verifies both.

- No raw colour outside `tokens.css` (a test enforces it). Components use the role aliases,
  never the `--n-*` ramp. To add a token, add it in schwarzwald.
- Charts are server-rendered SVG coloured by class, never inline.

<!-- BEGIN GENERATED: tokens -->
The role aliases in `tokens.css`: .

<!-- END GENERATED: tokens -->
