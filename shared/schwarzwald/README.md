# shared/schwarzwald

A copy of schwarzwald, written by its `sync.py`. Do not edit these files here.

- From: schwarzwald, synced 2026-11-08T03:13:06Z
- Parts: configs, fonts, guards, hooks, lint, rules, rust, tokens, tools
- Change anything: edit `source/` in schwarzwald, run `make deploy` there and commit the
  result here. A fix found here travels to schwarzwald as a patch.
- Verify this copy: `python3 shared/schwarzwald/check.py`, which needs no schwarzwald checkout.
  `manifest.json` lists a sha256 per file and the files that carry the exec bit.

## Upgrade notes

Breaking commits since the last sync that touch these parts:

- fix(tools)!: land.sh single mode runs the landing gate on the landed result before main moves
- fix(hooks)!: the landings-only refusal binds only the checkout marked with landing.target, never a lane clone
