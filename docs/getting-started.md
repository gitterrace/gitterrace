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
git terrace          # serves on http://127.0.0.1:4242, open it in your browser
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

Upgrade the same way you installed:

<!-- BEGIN GENERATED: upgrade -->
```bash
curl -fsSL https://gitterrace.dev/install.sh | sh
brew update && brew upgrade gitterrace
cargo install --locked --force --git https://github.com/gitterrace/gitterrace gitterrace
```

<!-- END GENERATED: upgrade -->

A server that is already running keeps serving the old version, since
`git terrace` joins it rather than replacing it. Stop it (Ctrl-C in its
terminal) and run `git terrace` again. When a release changes the cache
format, gitterrace builds a fresh cache in `.git/gitterrace/` the first time it
opens each repository, in place of the old one. There is nothing to migrate or
clean up.

## Troubleshooting

- **Port in use:** stop the other server, or pass `--addr 127.0.0.1:0` for a
  free port.
- **A setting has no effect:** `git terrace config` prints every value and the
  file it came from.
- **"A newer gitterrace owns this cache":** run the newer binary, or delete
  `.git/gitterrace/` to rebuild for this one.
- **Insights show indexing:** the cache is being built or was unreadable and is
  being rebuilt. Code and history pages work meanwhile.
