# Getting started

Install gitterrace, open your first repository, and upgrade or remove it later.

## Install

<!-- BEGIN GENERATED: install -->
```bash
brew install gitterrace/tap/gitterrace
curl --proto '=https' --tlsv1.2 -fsSL https://gitterrace.dev/install.sh | sh
cargo install --locked --git https://github.com/gitterrace/gitterrace gitterrace
```

Installs `git-terrace`, which git runs as `git terrace`.

<!-- END GENERATED: install -->

<!-- BEGIN GENERATED: msrv -->
Building with cargo needs Rust 1.96 or newer.

<!-- END GENERATED: msrv -->

### Verifying downloads

The shell installer checks each archive against its `.sha256` file only when
`sha256sum` is on the path, and stock macOS does not ship it. On macOS,
install with Homebrew, which checks the digest in the formula.

The Windows PowerShell installer does not check a digest. Download the zip and
its `.sha256` file from the release page and compare them before unpacking:

<!-- BEGIN GENERATED: verify -->
```powershell
(Get-FileHash .\gitterrace-x86_64-pc-windows-msvc.zip).Hash
Get-Content .\gitterrace-x86_64-pc-windows-msvc.zip.sha256
```

<!-- END GENERATED: verify -->

Every release archive also carries a build provenance attestation:
`gh attestation verify <archive> --repo gitterrace/gitterrace`.

The binaries are built auditable, so `cargo audit bin` on the installed binary lists the
dependencies they were built from, and each release carries a CycloneDX SBOM
for the workspace crates.

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

## How it runs

There is nothing to set up. `git terrace` starts a small local web server on
`127.0.0.1` for as long as it runs, and Ctrl-C stops it. There is no database
or account, and nothing listens once it stops. Each later `git terrace` adds its
repository to the server already running.

That is all most people need. Two options build on it:

- Keep it running from login: [Run it as a service](service.md) installs a
  systemd user unit or a launchd agent, so the address always answers.
- Share without a running process: [`git terrace export`](online.md) writes
  static files for any host.

## Windows

Releases ship a zip. `git terrace install-handler` only prints the `reg add`
commands.

<!-- BEGIN GENERATED: windows-paths -->
Config lives in `%APPDATA%\gitterrace`, the repository list in
`%LOCALAPPDATA%\gitterrace`.

<!-- END GENERATED: windows-paths -->

## Upgrade

Upgrade the same way you installed:

<!-- BEGIN GENERATED: upgrade -->
```bash
brew update && brew upgrade gitterrace
curl --proto '=https' --tlsv1.2 -fsSL https://gitterrace.dev/install.sh | sh
cargo install --locked --force --git https://github.com/gitterrace/gitterrace gitterrace
```

<!-- END GENERATED: upgrade -->

A running server keeps the old version: stop it (Ctrl-C) and run `git terrace`
again. A new cache format rebuilds the cache by itself.

## Uninstall

1. If you ran `git terrace install-handler`, run
   `git terrace install-handler --uninstall` first.
2. Remove the binary the way you installed it: `brew uninstall gitterrace`,
   `cargo uninstall gitterrace`, or delete the binary from the folder the
   shell installer reported (`~/.cargo/bin` by default).
3. Delete what it wrote, if you want it gone too: the config folder, the
   repository list (`~/.config/gitterrace` and `~/.local/state/gitterrace`, or
   the Windows folders above) and `.git/gitterrace/` in each repository you
   served.

## Troubleshooting

- **Port in use:** stop the other server, or pass `--addr 127.0.0.1:0` for a
  free port.
- **A setting has no effect:** `git terrace config` prints every value and the
  file it came from.
- **"A newer gitterrace owns this cache":** run the newer binary, or delete
  `.git/gitterrace/` to rebuild for this one.
- **Insights show indexing:** the cache is being built or was unreadable and is
  being rebuilt. Code and history pages work meanwhile.
