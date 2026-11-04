# gitterrace: Agent Guide

Local, read-only git browser with insights. [Architecture](docs/architecture.md).

## Layout

<!-- BEGIN GENERATED: crates -->
| Crate | Owns |
| --- | --- |
| `gitterrace-git` | All git reads through gix, pinned to commits. The worktree is read only in worktree.rs. |
| `gitterrace-index` | Insights cache in .git/gitterrace/, append-only and rebuildable, and the aggregates over it. |
| `gitterrace-config` | config.toml: the settings table, loader, validation and where each value came from. |
| `gitterrace-actions` | User actions: actions.toml, the payload v1 contract and the sandboxed runner. |
| `gitterrace-api` | JSON API (/api/v1) and the read-only MCP server, from one endpoint registry. |
| `gitterrace-cards` | The year-in-review SVG card. |
| `gitterrace-web` | axum server, askama templates, static assets and the request guard. |
| `gitterrace` | A local, read-only git browser: code, history and insights, offline, in one binary. Binary `git-terrace`. |
| `gitterrace-testkit` | Test support shared by every crate: the git helper, the fixture repository builder and the request helper. Dev-dependency only. |
| `schwarzwald` | The theme as typed constants |

<!-- END GENERATED: crates -->

Dependencies point down (diagram in architecture). Nothing below `web` knows HTTP. Nothing
outside `gitterrace-git` imports `gix`.

## Rules

- Read-only: no shelling out to `git` outside test fixtures. The worktree is read only in
  `gitterrace-git/src/worktree.rs`.
- Every link goes through `fmt::href` (templates: `c.base`, JS: the `gt-base`
  meta), so a repo works at `/` and at `/r/{name}/`. A crawler test fails on a
  link that escapes its repo.
- Every new insights page uses the shared `Filter` and the page memo
  (`cached` in `insights.rs`). Add unfiltered pages to `WARM_PAGES` in `lib.rs`.
- No inline scripts beyond the hashed theme snippet: the CSP refuses them. JS
  lives in `static/`.
- Never edit `theme/schwarzwald` or the generated block in `tokens.css`. `make theme` fails on
  drift. No raw color outside `tokens.css`.
- Logic lives in Rust. Templates render values, they do not compute them.
- Docs: `api.md`, `configuration.md` and every `GENERATED` block come from code registries
  (`ENDPOINTS`, `INSIGHTS`, config keys, `actions::schema`, `web::sitemap`, `CACHE_FILES`,
  clap `Cli`, Cargo.toml, script headers). Change the registry, `make docs-gen`, never edit a
  block. Prose holds decisions, not inventories.

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

- Gate: `make check` (theme, actions, docs-check, fmt, clippy `-D warnings`, lint, test, doc).
  `make lint` runs typos, vale, gitleaks, markdownlint, shellcheck, shfmt, ruff, actionlint,
  ast-grep and cargo-machete.
  `cargo test -p <crate>` while working on one crate. `make preview` restarts the
  preview on the default address.
- Commits: `type(scope): subject`, scope is the crate without `gitterrace-`. Commit by
  pathspec, never `git add -A`.
- One change, one commit. A change that spans several files or steps (a scaffold, a type moved
  before its callers, a refactor) is built in its own worktree or clone, never in the shared
  checkout, and lands on main as one squashed commit with its tests, docs and golden update.
  Main never carries the partial steps. `scripts/land-check.sh --base <main>` must pass on the
  squashed commit before it lands.
- Names: files in `scripts/` are `<area>-<verb>.<ext>`, make targets are `<area>[-<verb>]`,
  harness-only env vars use a `TEST_` prefix. `make names` enforces it.
