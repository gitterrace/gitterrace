# schwarzwald/kit

A copy of schwarzwald, written by its `sync.py`. Do not edit these files here.

- From: schwarzwald, synced 2026-11-08T21:08:11Z
- Parts: configs, fonts, guards, hooks, lint, rules, rust, tokens, tools
- Change anything: edit `source/` in schwarzwald, run `make deploy` there and commit the
  result here. A fix found here travels to schwarzwald as a patch.
- Verify this copy: `python3 schwarzwald/kit/check.py`, which needs no schwarzwald checkout.
  `manifest.json` lists a sha256 per file and the files that carry the exec bit.

## Upgrade notes

Breaking commits since the last sync that touch these parts:

- tools!: the kit lives under schwarzwald/ (kit, config, styles)
- guards!: the kit lives under schwarzwald/ (kit, config, styles)
- hooks!: the kit lives under schwarzwald/ (kit, config, styles)
- lint!: the kit lives under schwarzwald/ (kit, config, styles)
- guards!: one no_silent_failure rule, with pipe and substitution clauses
- guards!: config reads two forms only, NAME="literal" and NAME="${NAME:-literal}"
