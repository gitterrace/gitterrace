# gitterrace: Agent Guide

Local, read-only git browser with insights.
[Architecture](docs/architecture.md).

Shared rules: `schwarzwald/kit/rules/rules.md`. They apply here in full.

## Layout

<!-- BEGIN GENERATED: crates -->
| Crate | Owns |
| --- | --- |
| `gitterrace-types` | The types every crate shares: config values, git kinds and the JSON API shapes, each defined once. |
| `gitterrace-git` | All git reads through gix, pinned to commits. The worktree is read only in worktree.rs. |
| `gitterrace-index` | Insights cache in .git/gitterrace/, append-only and rebuildable, and the aggregates over it. |
| `gitterrace-config` | config.toml: the settings table, loader, validation and where each value came from. |
| `gitterrace-actions` | User actions: actions.toml, the payload v1 contract and the sandboxed runner. |
| `gitterrace-api` | JSON API (/api/v1) and the read-only MCP server, from one endpoint registry. |
| `gitterrace-web` | axum server, askama templates, static assets and the request guard. |
| `gitterrace` | A local, read-only git browser: code, history and insights, offline. Binary `git-terrace`. |
| `gitterrace-testkit` | Test support shared by every crate: the git helper, the fixture repository builder and the request helper. Dev-dependency only. |

<!-- END GENERATED: crates -->

Dependencies point down (diagram in architecture). Nothing below `web` knows
HTTP. Nothing outside `gitterrace-git` imports `gix`.

## Rules

- Read-only: no shelling out to `git` outside test fixtures. The worktree is
  read only in `gitterrace-git/src/worktree.rs`.
- Every link goes through `fmt::href` (templates: `c.base`, JS: the `gt-base`
  meta), so a repo works at `/` and at `/r/{name}/`. A crawler test fails on a
  link that escapes its repo.
- Every new insights page uses the shared `Filter` and the page memo (`cached`
  in `gitterrace-web/src/insights.rs`). Add unfiltered pages to `WARM_PAGES`
  in `gitterrace-web/src/lib.rs`.
- No inline scripts beyond the hashed theme snippet: the CSP refuses them. JS
  lives in `static/`.
- Never edit `schwarzwald/kit` or the generated block in `tokens.css`.
  `make theme` fails on drift. No raw color outside `tokens.css`.
- Logic lives in Rust. Templates render values, they do not compute them.
- Docs: `api.md`, `configuration.md` and every `GENERATED` block come from code
  registries (`ENDPOINTS`, `INSIGHTS`, config keys, `actions::schema`,
  `web::sitemap`, `CACHE_FILES`, clap `Cli`, Cargo.toml, script headers). Change
  the registry, `make docs-gen`, never edit a block. Prose holds decisions, not
  inventories.

## Code conventions

- Imports in three groups: std, external, `crate::`. No `use super::` outside
  `mod tests`.
- Start private, widen to `pub(crate)`, `pub` only for the crate API in
  `lib.rs`.
- `thiserror` in library crates, `anyhow` only in the binary.
- No `unwrap()` outside tests. No `let _ = fallible()`.
- Declaration order: mod, use, const, types, impl (constructor first), fns,
  `mod tests` last.
- Comments: ASCII, explain why, never narrate the next line.
- Tests: unit tests at the bottom of the file. Repo fixtures are built in a
  `tempfile` dir, never against a real checkout.

## Workflow

- Gate: `make check` runs every gate in the order CI does: gc-guard, theme,
  actions, send-vectors, names, lines, bench-guard, docs-check, fmt, clippy
  `-D warnings`, lint, test, doc. `make help` lists only the targets a person
  runs, the steps are in the `check:` line of the Makefile. `make lint` runs
  the linters in [CONTRIBUTING](CONTRIBUTING.md#lint) and the provenance
  checks below. Without the shared harness kit checkout (`HARNESS_KIT`) its
  link check is skipped with a note.
  `cargo test -p <crate>` while working on one crate.
  `make preview` restarts the preview on the default address.
- Commits: `type(scope): subject`, scope is the crate without `gitterrace-`.
  Commit by pathspec, never `git add -A`. The message says what changed and how
  it is tested, in plain words. `git config core.hooksPath scripts/hooks` runs
  the message rules on every commit.
- One change, one commit, squashed in its own clone or worktree (shared rules,
  Commits). The gate for a squashed commit here is
  `scripts/land-check.sh --base <main>`.
- Accepting commits written without a build:
  `python3 scripts/lint/commits-check.py --range <base>..<tip>` prints the
  files and diffstat and runs the cargo-free gates those file types need,
  with the message step when the tip is checked out. `scripts/land-check.sh`
  still builds and tests afterwards.
- Names: files in `scripts/` sit in an area folder (`lint`, `verify`, `bench`,
  `dev`, `lib`, `release`) and are `<area>-<verb>.<ext>`. `land-check.sh` is
  the one entry point at the top, and `hooks`, `guards` and `ast-grep` hold
  data other tools name. Make targets are `<area>[-<verb>]`, harness-only env
  vars use a `TEST_` prefix. `make names` enforces it.

## Provenance stays out

Task ids, plans and who did what belong on the tracker, never in the tree or a
commit message. `make lint` enforces it on the tree, land-check on the messages
it lands. Real names, hosts and ids for the checks go in the gitignored
`.personal-denylist` (one case-insensitive regex per line), never in the
repository.

- `CONV-NO-INTERNAL-REFS`, `CONV-PERSONAL-DATA`, `CONV-NO-OWN-SHAS`: the shared
  lints in `schwarzwald/kit/lint`, with gitterrace's values in
  `schwarzwald/config/lint.toml` and the refused words in
  `schwarzwald/kit/rules/deny.toml`. Each runs on its red and green case in
  `make lint`. Design docs (the `design` scope there) hold product design
  only: no work breakdown, owner or estimate section.
- `test-names-describe-behavior` (ast-grep): a test is named by its behavior,
  not a label.
- `scripts/hooks/commit-msg`: execs the shared hook, the subject form plus the
  message rules above, before a commit exists. `make lint` runs the shared hook
  test through the shim.
- Tool-use guards: `scripts/guards/adapters/*.sh` exec the shared guards
  (`schwarzwald/kit/guards`), with gitterrace's values in
  `schwarzwald/config/guards.env`. `make lint` runs their selftest and adapters,
  the red probes run with
  `GUARDS_SELFTEST_PRODUCT=$PWD bash schwarzwald/kit/guards/tests/red.sh`.
- Allowlists and counts only fall: `schwarzwald/config/ratchet.toml`, checked by
  `schwarzwald/kit/tools/ratchet-check.py` in land-check.
