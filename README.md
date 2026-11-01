<h1 align="center">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/brand/readme-header-dark.svg">
  <img alt="gitterrace" src="docs/brand/readme-header-light.svg" width="600">
</picture>
</h1>

<p align="center"><b>Your repository's history, step by step. Run <code>git terrace</code> and read it in your browser.</b></p>

Browse any git repository in your browser, offline, read-only: code, commits, blame,
branches, worktrees, stashes, live updates and insights. No database, accounts or network.

## Install

```bash
brew install timorunge/tap/gitterrace   # or: cargo install --git https://github.com/timorunge/gitterrace gitterrace
```

<!-- BEGIN GENERATED: msrv -->
Rust 1.96 or newer.

<!-- END GENERATED: msrv -->
<!-- BEGIN GENERATED: install -->
Installs `git-terrace` and `gitterrace` for one program; git runs `git-terrace` as `git terrace`.

<!-- END GENERATED: install -->
Windows: use WSL.
## Commands

<!-- BEGIN GENERATED: commands -->
```
git terrace [paths]       Browse a git repository in your browser, offline [--addr --poll --no-watch --config --no-actions]
git terrace serve         Serve the repository in the browser (the default)
git terrace config        Print the effective configuration and where each value comes from
git terrace mcp           Run a read-only MCP server over stdio for agents
git terrace wrapped       Write the year-in-review card for the repository [--year --layout --theme --out]
git terrace export <out>  Write a static copy of the site to a folder, for sharing or hosting [--commits --max-file-kb]
```

<!-- END GENERATED: commands -->

<!-- BEGIN GENERATED: actions -->
Run your own program on a selection (`line`, `file`, `commit`, `hunk`, `blame`, `search-hit`, `insight`, `page`). Examples in `docs/actions/`: `clipboard.sh`, `editor.sh`, `obsidian.sh`.

<!-- END GENERATED: actions -->
Off until you write `actions.toml`: [actions](docs/actions.md).

Docs: [configuration](docs/configuration.md), [JSON API](docs/api.md),
[architecture](docs/architecture.md), [design](docs/design.md). Binds to `127.0.0.1`, never
writes to your repository. MIT licensed.
