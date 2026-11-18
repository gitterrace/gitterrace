<h1 align="center">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/brand/readme-header-dark.svg">
  <img alt="gitterrace" src="docs/brand/readme-header-light.svg" width="600">
</picture>
</h1>

<!-- BEGIN GENERATED: pitch -->
A local, read-only git browser: code, history and insights, offline.

<!-- END GENERATED: pitch -->

Code, commits, blame, branches, worktrees, stashes and insights, with no
database, accounts or network ([why](https://gitterrace.dev/docs/why/)).

<!-- BEGIN GENERATED: demo -->
Try it without installing: [the live demo](https://demo.gitterrace.dev): jq
1.8.2, ripgrep 15.2.0 and gitterrace v0.1.1, exported.

[The screenshots](docs/screenshots/) show the open source projects
[jq](https://github.com/jqlang/jq/tree/jq-1.8.2) and
[ripgrep](https://github.com/BurntSushi/ripgrep/tree/15.2.0), used under their
MIT licenses. They are not affiliated with gitterrace. Thanks to their authors
for great tools.

<!-- END GENERATED: demo -->

<!-- BEGIN GENERATED: shots -->
<picture>
<source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/overview-dark.webp">
<img src="docs/screenshots/overview-light.webp" alt="Browse: the repository page with README, last commit, file list and key numbers.">
</picture>

<!-- END GENERATED: shots -->

## Install

<!-- BEGIN GENERATED: install -->
```bash
curl --proto '=https' --tlsv1.2 -fsSL https://gitterrace.dev/install.sh | sh
brew install gitterrace/tap/gitterrace
cargo install --locked --git https://github.com/gitterrace/gitterrace gitterrace
```

Installs `git-terrace`, which git runs as `git terrace`.

<!-- END GENERATED: install -->

<!-- BEGIN GENERATED: msrv -->
Building with cargo needs Rust 1.96 or newer.

<!-- END GENERATED: msrv -->

## Quickstart

<!-- BEGIN GENERATED: quickstart -->
```bash
cd path/to/any/git/repo
git terrace          # serves on http://127.0.0.1:4242, open it in your browser
cd ../another/repo
git terrace          # joins the server already running, no second port
```

<!-- END GENERATED: quickstart -->

Next: [getting started](https://gitterrace.dev/docs/getting-started/) and
[using it](https://gitterrace.dev/docs/using/). Docker serves read-only at
`http://localhost:8080`
([run with Docker](https://gitterrace.dev/docs/online/#run-with-docker)).

## Send a finding with its context

Review a change before you push it, then send each finding on: back to an
agent, or as a task or ticket. Open **Working tree** (`/worktree`) for your
uncommitted changes, or any file, diff or commit. Select the lines, press `.`
and pick a **Send to** target. The note carries the marked source lines and
ends with a line agents and scripts can parse:

```text
source: <repo>@<sha>:<path>#L<a>-L<b>
```

`<sha>` is the full commit id, so the reference holds after more commits.
Without a target, **Copy as markdown** puts the same note on the clipboard.

## Commands

<!-- BEGIN GENERATED: commands -->
```text
git terrace [paths]          Browse a git repository in your browser, offline [--addr --no-watch --config --no-actions --public]
git terrace serve            Serve the repository in the browser (the default)
git terrace config           Print the effective configuration and where each value comes from
git terrace mcp              Run a read-only MCP server over stdio for agents [--served]
git terrace open <link>      Open a `gitterrace://` or `/go` permalink in the browser, starting a server if needed [--print]
git terrace remove [target]  Take a repository off the running server. Its pages then answer 404
git terrace install-handler  Register (or with `--uninstall` remove) this program as the `gitterrace://` link handler [--uninstall]
git terrace init             Write a documented `config.toml` and the pre-filled `actions.toml` to the config folder
git terrace export [out]     Write a static copy of the site to a folder, for sharing or hosting [--force --commits --file-kb --export-pages --quiet --project]
```

<!-- END GENERATED: commands -->

<!-- BEGIN GENERATED: actions -->
Run your own program on a selection (`line`, `file`, `commit`, `hunk`, `blame`,
`search-hit`, `insight`, `page`). Examples in `docs/actions/`: `clipboard.sh`,
`editor.sh`, `obsidian.sh`.

<!-- END GENERATED: actions -->

Or send it to a tracker, notes app or mail draft. Both are off until you write
`actions.toml` ([set up](https://gitterrace.dev/docs/actions/)).

gitterrace binds to `127.0.0.1` and never changes your files or history
([security model](https://gitterrace.dev/docs/security/)). All docs:
[gitterrace.dev/docs](https://gitterrace.dev/docs/).

## Contributing

Issues and pull requests are welcome: see [contributing](CONTRIBUTING.md).
Report vulnerabilities privately ([SECURITY.md](SECURITY.md)).

## License

<!-- BEGIN GENERATED: license -->
MIT, see [LICENSE](LICENSE).

<!-- END GENERATED: license -->
