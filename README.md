<h1 align="center">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/brand/readme-header-dark.svg">
  <img alt="gitterrace" src="docs/brand/readme-header-light.svg" width="600">
</picture>
</h1>

<p align="center"><b>Your repository's history, step by step. A GitHub-style view of any git repo, offline, in one binary. Run <code>git terrace</code>.</b></p>

Browse any git repository in your browser, offline: code, commits, branches, blame, rendered
markdown, and insights that go past what GitHub and GitLab show. Read-only and local, with no
database, no accounts and no network.

```bash
git terrace                       # the repo you are in, on 127.0.0.1:4242
git terrace ~/src/app ~/src/lib   # several repos on one port
git terrace ~/src                 # every repo in a folder
```

## Install

```bash
brew install timorunge/tap/gitterrace
curl -LsSf https://github.com/timorunge/gitterrace/releases/latest/download/gitterrace-installer.sh | sh
cargo install --git https://github.com/timorunge/gitterrace gitterrace
```

<!-- BEGIN GENERATED: msrv -->
Rust 1.96 or newer.

<!-- END GENERATED: msrv -->

<!-- BEGIN GENERATED: install -->
Every route installs 2 names for one program: `git-terrace` and `gitterrace`. git runs `git-terrace` as `git terrace`.

<!-- END GENERATED: install -->

Windows: run gitterrace under WSL; native Windows is not supported (the scripts detect WSL).

## What you get

- **Several repos** on one port, with a switcher.
- **Full git view**: files, blame, commits, branches, tags, worktrees, stashes, uncommitted
  changes, and a banner for a detached HEAD or an operation in progress.
- **Live**: pages refresh when the repository changes.
- **Insights** on one shared filter (type, scope, author, path, period). `?rev=` follows any ref.
- **Actions**: send a selection to your own script. Off until you write `actions.toml`.
- **Export** a static site, **wrapped** year card, **MCP** server for agents.

`?` in the browser lists every key.

## Commands

<!-- BEGIN GENERATED: commands -->
```
git terrace [paths]       A GitHub/GitLab-style view of a git repository, offline [--addr --poll --no-watch --config --no-actions]
git terrace serve         Serve the repository in the browser (the default)
git terrace config        Print the effective configuration and where each value comes from
git terrace mcp           Run a read-only MCP server over stdio for agents
git terrace wrapped       Write the year-in-review card for the repository [--year --layout --theme --out]
git terrace export <out>  Write a static copy of the site to a folder, for sharing or hosting [--commits --max-file-kb]
```

<!-- END GENERATED: commands -->

`git terrace config` prints every setting and where it came from. Keys and defaults:
[configuration](docs/configuration.md). JSON API: [JSON API](docs/api.md).

## Actions

<!-- BEGIN GENERATED: actions -->
Built in, no configuration: **Copy permalink** and **Copy as markdown**. Your own actions run a program of yours on a selection, offered on `line`, `file`, `commit`, `hunk`, `blame`, `search-hit`, `insight`, `page`. Examples in [`docs/actions/`](docs/actions/):

- [`clipboard.sh`](docs/actions/clipboard.sh): Copy the selection as markdown.
- [`editor.sh`](docs/actions/editor.sh): Open the file at the selected line in the editor.
- [`obsidian.sh`](docs/actions/obsidian.sh): Send the selection to Obsidian.
- [`tendir.sh`](docs/actions/tendir.sh): Send to tendir.
- [`webhook.sh`](docs/actions/webhook.sh): POST the payload as-is.

<!-- END GENERATED: actions -->

Keys, payload and the security model: [actions](docs/actions.md).

## Safe by default

Binds to `127.0.0.1`, refuses foreign Host headers and cross-site requests, strict CSP. It
never changes your history, refs or working tree; its only write is a disposable cache in
`.git/gitterrace/`. Actions are the one state-changing path: off until you write an
`actions.toml` in your own config folder, which a repository cannot supply.

[Architecture](docs/architecture.md), [design](docs/design.md). MIT licensed.
