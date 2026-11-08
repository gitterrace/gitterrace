# shared/schwarzwald

A copy of schwarzwald, written by its `sync.py`. Do not edit these files here.

- From: schwarzwald, synced 2026-11-08T12:59:36Z
- Parts: configs, fonts, guards, hooks, lint, rules, rust, tokens, tools
- Change anything: edit `source/` in schwarzwald, run `make deploy` there and commit the
  result here. A fix found here travels to schwarzwald as a patch.
- Verify this copy: `python3 shared/schwarzwald/check.py`, which needs no schwarzwald checkout.
  `manifest.json` lists a sha256 per file and the files that carry the exec bit.

## Upgrade notes

Breaking commits since the last sync that touch these parts:

- lint!: read the denylist refresh from a private argv file, never a tracked shell string (SL-1)
- lint!: drop [history] keep/sha_fields/bundles/replace/skip, [run] shfmt_indent and markdown_exclude; history-scrub reads its own history-scrub.toml
- configs!: drop prettier-base.json and markdownlint-base.jsonc, remove stray common testing.rs
- common!: drop testing feature and tmp(), make buildinfo helpers and sync_dir private
