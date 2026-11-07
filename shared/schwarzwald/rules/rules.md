# Rules

The rules every product follows. They hold for everything that lands: code,
comments, tests, docs, generated pages, Makefile help, lint rules, and the
subject and body of every commit. A product's own `AGENTS.md` adds its product
rules on top and links this file. Nothing here names a product: each product
names its own gate command, crate prefix and make targets.

## What lands carries no provenance

A reader of the tree has the tree and its history, nothing else. A reference
they cannot open reads as authority and tells them nothing. Say what the
referenced thing says instead.

1. No tracker ids, not even in an ignore reason or a "see ..." note. An ignore
   reason says why in product terms.
2. No planning labels: numbered waves, phases, plans or steps, and short label
   codes such as a capital letter and a number.
3. No process words: who does the work, who approves it, and what has to land
   first.
4. No personal names, agent names, hostnames or local paths. Write `$HOME` or
   `~`.
5. No commit of the repository's own history by sha. A rewrite makes it dead.
   Say what the commit did. Upstream shas (a pinned corpus, a vendored release)
   are fine.
6. Test names describe behavior (`owner_ties_follow_names`), never a label.
7. A commit message describes the product change and its tests in plain words. A
   golden change is described by what changed ("12 ownership cells: only the
   order of tied owners").
8. Design docs in the repository are product design only: no work breakdown,
   owners or estimates.
9. No em dashes. A comma, a colon or a new sentence does the job.

`deny.toml` holds the shapes and words as patterns, each with lines it must
refuse and lines it must pass. The shared lints (`lint/internal-refs.py`,
`lint/personal-data.py`, `lint/own-shas.py`) read it, and each product's message
gate runs them on every commit it lands. Real names and real ids never get
committed: they go in the product's gitignored `.personal-denylist`, one
case-insensitive regex per line.

Tracker threads and direct messages may carry ids and labels. That is where they
belong.

## Commits

- Work in your own clone or worktree, never in a checkout someone else uses.
- One change, one commit. The tests, guard, docs, golden update and formatting
  fixes of a change go into that change's commit.
- No `fixup!`, `squash!`, `wip`, style-only, "satisfy clippy", "re-bless" or
  "make it build" commits. The landing tool refuses those subjects (`deny.toml`
  `[partial_subject]`).
- A change that needs several steps (a scaffold, a type moved before its
  callers) stays in your clone until it is complete, then lands as one squashed
  commit. Main never carries partial steps.
- Subject form `type(scope): subject`. The product names its types and its scope
  rule (often the crate name without the product prefix).
- Commit by pathspec, never `git add -A`.
- Before asking for a landing: rebase onto the newest main, then run the
  product's gate on the squashed commit. It must exit 0.
- Landing restamps author and committer date to the landing time and keeps the
  message and identity.

## The shared tree always compiles

Main builds and passes its gate after every commit. A commit that leaves main
red is reverted, not fixed forward with a second commit. A test that cannot pass
yet is not committed red: it is ignored with a reason in product terms, or it
waits in the clone with its change.

## Names

- Tracked files have plain lower-case names: no `final`, `old`, `new`, `tmp`,
  `copy`, `v2`, capitals or spaces. Conventional names (`README.md`, `Makefile`,
  `Cargo.toml`, licence files) are exempt.
- Scripts are `<area>-<verb>.<ext>`. Make targets are `<area>[-<verb>]`.

## Code style

- Imports in three groups: std, external, `crate::`. No `use super::` outside
  `mod tests`.
- Start private, widen to `pub(crate)`, `pub` only for the crate API in
  `lib.rs`.
- `thiserror` in library crates, `anyhow` only in binaries.
- No `unwrap()` outside tests. No `let _ = fallible()`.
- Declaration order: mod, use, const, types, impl (constructor first), fns,
  `mod tests` last.
- Comments: ASCII, explain why, never narrate the next line.
- Tests: unit tests at the bottom of the file. Repository fixtures are built in
  a temporary directory, never against a real checkout.
- Scripts: Python 3.11 or newer with the standard library only, bash for shell.
  Every gate check proves itself on a planted defect before it trusts a pass.

## Copies are never edited

A vendored or synced copy (`shared/`, a vendored crate, a vendored stylesheet)
is never edited in the product. Change its source, regenerate, and commit the
new copy. The product's gate fails on drift. Docs that list registries (rules,
targets, parts) are generated from the registry or checked against it, never
kept by hand.
