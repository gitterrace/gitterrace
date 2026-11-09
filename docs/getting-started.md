# Getting started

Install gitterrace, open a repository in the browser, and keep it up to date.

## Install

<!-- BEGIN GENERATED: install -->
```bash
curl -fsSL https://gitterrace.dev/install.sh | sh
brew install gitterrace/tap/gitterrace
cargo install --locked --git https://github.com/gitterrace/gitterrace gitterrace
```

Installs `git-terrace`; git runs it as `git terrace`.

<!-- END GENERATED: install -->

<!-- BEGIN GENERATED: msrv -->
Building with cargo needs Rust 1.96 or newer.

<!-- END GENERATED: msrv -->

## Quickstart

<!-- BEGIN GENERATED: quickstart -->
```bash
cd path/to/any/git/repo
git terrace          # serves on 127.0.0.1 and opens your browser
cd ../another/repo
git terrace          # joins the server already running, no second port
```

<!-- END GENERATED: quickstart -->

Every repository lives under `/r/<name>/`, also the first, so its links stay
valid as others join. `git terrace remove [path|name]` takes one off again.
[Using it](using.md) covers serving a folder of repositories, the log graph,
outlines, keys and the settings page. [Command line](cli.md) lists every command
and flag.

## Windows

Releases ship a zip. Config lives in `%APPDATA%\gitterrace` and the repository
registry in `%LOCALAPPDATA%\gitterrace`. `git terrace install-handler` prints
the `reg add` commands instead of writing the registry, so you can read them
first.

## Upgrade

Run the same install command again. The insights cache in `.git/gitterrace/` is
disposable: a new format is rebuilt on first open, and deleting the folder is
always safe.

## Troubleshooting

- **Port in use:** stop the other server, or pass `--addr 127.0.0.1:0` for a
  free port.
- **A setting has no effect:** `git terrace config` prints every value and the
  file it came from.
- **"A newer gitterrace owns this cache":** run the newer binary, or delete
  `.git/gitterrace/` to rebuild for this one.
- **Insights show indexing:** the cache is being built or was unreadable and is
  being rebuilt. Code and history pages work meanwhile.
