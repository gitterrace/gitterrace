# schwarzwald/kit

A copy of schwarzwald, written by its `sync.py`. Do not edit these files here.

- From: schwarzwald, synced 2026-11-09T12:20:45Z
- Parts: configs, fonts, guards, hooks, lint, rules, rust, tokens, tools
- Change anything: edit `source/` in schwarzwald, run `make deploy` there and commit the
  result here. A fix found here travels to schwarzwald as a patch.
- Verify this copy: `python3 schwarzwald/kit/check.py`, which needs no schwarzwald checkout.
  `manifest.json` lists a sha256 per file and the files that carry the exec bit.

## Upgrade notes

Breaking commits since the last sync that touch these parts:

- fix(land)!: a gate change is judged by the candidate's gate, refusals are remembered, one gate per clone
