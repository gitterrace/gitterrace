# Getting started

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
git terrace          # serves on http://127.0.0.1:4242, open it in your browser
cd ../another/repo
git terrace          # joins the server already running, no second port
```

<!-- END GENERATED: quickstart -->

Next: [using it](using.md) and the [command line](cli.md).

## Windows

Releases ship a zip. Config lives in `%APPDATA%\gitterrace`, the repository
list in `%LOCALAPPDATA%\gitterrace`. `git terrace install-handler` only prints
the `reg add` commands.

## Upgrade

Upgrade the same way you installed:

<!-- BEGIN GENERATED: upgrade -->
```bash
curl -fsSL https://gitterrace.dev/install.sh | sh
brew update && brew upgrade gitterrace
cargo install --locked --force --git https://github.com/gitterrace/gitterrace gitterrace
```

<!-- END GENERATED: upgrade -->

A running server keeps the old version: stop it (Ctrl-C) and run `git terrace`
again. A new cache format rebuilds the cache by itself.

## Troubleshooting

- **Port in use:** stop the other server, or pass `--addr 127.0.0.1:0` for a
  free port.
- **A setting has no effect:** `git terrace config` prints every value and the
  file it came from.
- **"A newer gitterrace owns this cache":** run the newer binary, or delete
  `.git/gitterrace/` to rebuild for this one.
- **Insights show indexing:** the cache is being built or was unreadable and is
  being rebuilt. Code and history pages work meanwhile.
