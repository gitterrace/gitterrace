# gitterrace: Agent Guide

gitterrace is a local, read-only git browser with stats. One binary, run inside a
repository, serves `127.0.0.1:4242`. No database, no users, no comments.

## Layout

| Crate          | Owns                                                          |
| -------------- | ------------------------------------------------------------- |
| `gitterrace-git`   | All git reads via gix. Pinned to commits, never the worktree. |
| `gitterrace-index` | Stats cache in `.git/gitterrace/`, append-only, rebuildable.      |
| `gitterrace-web`   | axum server, askama templates, embedded static assets.        |
| `gitterrace`       | CLI binary: parse args, call `gitterrace_web::serve`.             |

Dependency direction is strictly downward: `gitterrace -> web -> index -> git`.
`gitterrace-web` may call `gitterrace-git` directly for browsing. Nothing below `web`
knows about HTTP; nothing outside `gitterrace-git` imports `gix`.

## Rules

- Read-only. gitterrace never writes to the repository except `.git/gitterrace/`.
  No shelling out to `git`.
- Browsing never waits on the index. Stats pages show progress until ready.
- Every repo read takes a resolved commit id, so a commit landing mid-request
  cannot tear a page. The one exception is the Uncommitted view, which reads
  the worktree read-only, lives only in `gitterrace-git/src/worktree.rs`, and must
  tolerate files changing or vanishing mid-read (skip, never fail the page).
- Crates: `gitterrace-cards` (shareable SVG/PNG cards, badges) and `gitterrace-api`
  (JSON API + MCP) sit beside `gitterrace-web` and depend only on git and index.
- The cache is disposable: deleting `.git/gitterrace/` must only cost a rebuild.
  Bump its format version instead of migrating.
- Logic lives in Rust. Templates render values, they do not compute them.

## Code conventions

- Imports in three groups: std, external, `crate::`. No `use super::` outside
  `mod tests`.
- Start private, widen to `pub(crate)`, `pub` only for the crate API in `lib.rs`.
- `thiserror` in library crates, `anyhow` only in the binary.
- No `unwrap()` outside tests. No `let _ = fallible()`.
- Declaration order: mod, use, const, types, impl (constructor first), fns,
  `mod tests` last.
- Comments: ASCII, explain why, never narrate the next line.
- Tests: unit tests at the bottom of the file. Repo fixtures are built in a
  `tempfile` dir, never against a real checkout.

## Workflow

