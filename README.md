<h1 align="center">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/brand/readme-header-dark.svg">
  <img alt="gitterrace" src="docs/brand/readme-header-light.svg" width="600">
</picture>
</h1>

<p align="center"><b>Your repository's history, step by step. Run <code>git terrace</code> and read it in your browser.</b></p>

Browse any git repository in your browser, offline, read-only: code, commits, blame,
branches, worktrees, stashes, live updates and insights. No database, accounts or network.
Agents write more of the code each month, and few people read their diffs. This is a calm
place to look again ([why](docs/why.md)).

<!-- BEGIN GENERATED: demo -->
Try it without installing: [the live demo](https://demo.gitterrace.dev) is this repository, exported.

<!-- END GENERATED: demo -->

## Install

<!-- BEGIN GENERATED: msrv -->
Rust 1.96 or newer.

<!-- END GENERATED: msrv -->
<!-- BEGIN GENERATED: install -->
```bash
brew install gitterrace/tap/gitterrace
cargo install --git https://github.com/gitterrace/gitterrace gitterrace
```

Installs `git-terrace` and `gitterrace` for one program; git runs `git-terrace` as `git terrace`.

<!-- END GENERATED: install -->
Windows: builds natively (`x86_64-pc-windows-msvc`), but the runtime is only tested on WSL so far. Use WSL until a native run is confirmed. Config lives in `%APPDATA%\gitterrace`, the repo registry in `%LOCALAPPDATA%\gitterrace`, and `install-handler` prints the `reg add` commands instead of writing the registry.

## Quickstart

```bash
cd path/to/any/git/repo
git terrace          # serves on 127.0.0.1 and opens your browser
```

## Commands

<!-- BEGIN GENERATED: commands -->
```
git terrace [paths]          Browse a git repository in your browser, offline [--addr --poll --no-watch --config --no-actions]
git terrace serve            Serve the repository in the browser (the default)
git terrace config           Print the effective configuration and where each value comes from
git terrace mcp              Run a read-only MCP server over stdio for agents
git terrace wrapped          Write the year-in-review card for the repository [--year --layout --theme --out]
git terrace open <link>      Open a `gitterrace://` or `/go` permalink in the browser, starting a server if needed [--print]
git terrace install-handler  Register (or with `--uninstall` remove) this program as the `gitterrace://` link handler [--uninstall]
git terrace init             Write a documented `config.toml` and the pre-filled `actions.toml` to the config folder
git terrace export <out>     Write a static copy of the site to a folder, for sharing or hosting [--commits --max-file-kb]
```

<!-- END GENERATED: commands -->

<!-- BEGIN GENERATED: actions -->
Run your own program on a selection (`line`, `file`, `commit`, `hunk`, `blame`, `search-hit`, `insight`, `page`). Examples in `docs/actions/`: `clipboard.sh`, `editor.sh`, `obsidian.sh`.

<!-- END GENERATED: actions -->
Send a selection to a tracker, notes app or mail draft, or run your own program on it. Off until you write `actions.toml`: [how to set it up](docs/actions.md).

Docs: [why](docs/why.md), [configuration](docs/configuration.md), [JSON API](docs/api.md),
[architecture](docs/architecture.md), [design](docs/design.md), [benchmarks](docs/benchmarks.md). Binds to `127.0.0.1`, never
writes to your repository. The one-page site is generated into [site/](site/index.html).

## Contributing

Issues and pull requests are welcome: see [contributing](CONTRIBUTING.md), and report
vulnerabilities privately as [SECURITY.md](SECURITY.md) describes. Built by AI coding agents
and humans together. A human sets the direction and decides ([why](docs/why.md)).

## License

<!-- BEGIN GENERATED: license -->
MIT, see [LICENSE](LICENSE).

<!-- END GENERATED: license -->
