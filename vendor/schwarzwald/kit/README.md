# vendor/schwarzwald/kit

A copy of schwarzwald, written by its `sync.py`. Do not edit these files here.

- From: schwarzwald, synced 2026-11-25T12:56:57Z
- Items: changelog-gen, clone, commit-paths, configs, cross-clippy, design-tokens, disk-floor, disk-sweep, fonts, golden, guards-claude, guards-engine, guards-jcode, guards-rules, help-mk, hook-commit-dates, hook-commit-msg, hook-pre-commit, hook-stale-index, kit-tests, land, lint, lint-run, markdownlint, ratchet, rules, theme-tokens, vale (with tests: commit-paths, golden, guards-engine, hook-commit-dates, hook-commit-msg, hook-stale-index, lint-run)
- Themes (the rest left out of tokens/theme-tokens.json): kandel
- Change anything: edit `source/` in schwarzwald, run `make deploy`
  there and commit the result here. A fix found here travels to schwarzwald
  as a patch.
- Verify this copy, which needs no schwarzwald checkout:
  `python3 vendor/schwarzwald/kit/check.py`
- `manifest.json` is the lock: the items, a sha256 per file and the files that
  carry the exec bit.
