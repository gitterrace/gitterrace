# Theme extension

The roles gitterrace needs beyond a task app (code rows and words, syntax,
selection, the heat ramp, the eight series, the window surfaces) were proposed
here and now live in schwarzwald. The rules, the solved values for all four
themes and the contrast floors are there:

- `schwarzwald/contract/extension-gitterrace.md`: the rule for each role.
- `theme/schwarzwald/theme-tokens.json`: the values, under `code.*`, `heat.*`,
  `series.*` and `surfaces.*`.

gitterrace does not solve colours. `scripts/gen-theme.py` reads the snapshot
and writes `tokens.css` and `palette.rs`. See [design.md](design.md).
