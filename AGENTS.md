# gitterrace: Agent Guide

gitterrace is a local, read-only git browser with insights. One binary serves
one or several repositories on `127.0.0.1:4242`. [Architecture](docs/architecture.md) has the
decisions; [design](docs/design.md) the theme pipeline.

## Layout

<!-- BEGIN GENERATED: crates -->
| Crate | Owns |
| --- | --- |
| `gitterrace-git` | All git reads through gix, pinned to commits. The worktree is read only in worktree.rs. |
| `gitterrace-index` | Insights cache in .git/gitterrace/, append-only and rebuildable, and the aggregates over it. |
| `gitterrace-config` | config.toml: the settings table, loader, validation and where each value came from. |
| `gitterrace-actions` | User actions: actions.toml, the payload v1 contract and the sandboxed runner. |
| `gitterrace-api` | JSON API (/api/v1) and the read-only MCP server, from one endpoint registry. |
| `gitterrace-cards` | SVG cards and badges, and the year-in-review card. |
| `gitterrace-web` | axum server, askama templates, static assets and the request guard. |
| `gitterrace` | A GitHub/GitLab-style view of any git repository, offline, in one binary Binaries `git-terrace` and `gitterrace`. |
| `schwarzwald` | The theme as typed constants |

<!-- END GENERATED: crates -->

Dependencies point down: `gitterrace -> web -> (api, cards, actions) -> index -> git`, with `config` beside them (see the diagram in [architecture](docs/architecture.md)).
Nothing below `web` knows about HTTP; nothing outside `gitterrace-git`
imports `gix`. Colours come only from the vendored `schwarzwald` crate in
`theme/schwarzwald/rust`.

## Rules

- Read-only. gitterrace never writes to the repository except `.git/gitterrace/`.
  No shelling out to `git` outside test fixtures.
- Browsing never waits on the index. Insights pages show progress until ready.
- Every repo read takes a resolved commit id, so a commit landing mid-request
  cannot tear a page. The one exception is the Uncommitted view, which reads
  the worktree read-only, lives only in `gitterrace-git/src/worktree.rs`, and must
  tolerate files changing or vanishing mid-read (skip, never fail the page).
- Every link goes through `fmt::href` (templates: `c.base`, JS: the `gt-base`
  meta), so a repo works at `/` and at `/r/{name}/`. A crawler test fails on a
  link that escapes its repo.
- Every new insights page uses the shared `Filter` and the page memo
  (`cached` in `insights.rs`). Add unfiltered pages to `WARM_PAGES` in `lib.rs`.
- No inline scripts beyond the hashed theme snippet: the CSP refuses them. JS
  lives in `static/`.
- Colours and fonts come from `theme/schwarzwald`, a snapshot written by
  schwarzwald's `sync.py`. Never edit it or the generated block in
  `tokens.css`; `make theme` fails on drift. No raw colour outside `tokens.css`.
- The cache is disposable: deleting `.git/gitterrace/` must only cost a rebuild.
  Bump its format version instead of migrating.
- Logic lives in Rust. Templates render values, they do not compute them.
- Generated docs: `docs/api.md`, `docs/configuration.md` and the `<!-- BEGIN GENERATED: id -->`
  blocks in README, AGENTS, `docs/actions.md`, `architecture.md` and `design.md`. Facts live in
  code registries (`ENDPOINTS`, `INSIGHTS`, config keys, `gitterrace_actions::schema`,
  `gitterrace_web::sitemap`, `CACHE_FILES`, the clap `Cli`, Cargo.toml, `docs/actions/*.sh`
  headers). Change the registry, run `make generate`, never edit a block. `make check` fails on
  drift, a block nobody fills, and a registry that disagrees with the code it describes. Prose
  stays by hand and holds decisions, not inventories.

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

