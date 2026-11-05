# Contributing

Thanks for taking a look. If you spot something, open an issue or a pull request.

gitterrace is built by AI coding agents and humans together: agents write and review most
of the code, and a human sets the direction and decides what goes in
([why](docs/why.md)). Contributions from people are just as welcome. The rules below apply
to everyone, and most of them are checks that run, not prose to remember.

## License

gitterrace is MIT licensed (see [LICENSE](LICENSE)). A pull request is offered under the
same terms.

## Setup

You need a Rust toolchain (the version is pinned in `rust-toolchain.toml`), git and
Python 3. `uv` is only needed to regenerate the README header tagline.

```sh
git clone https://github.com/gitterrace/gitterrace && cd gitterrace
make build
make install  # puts git-terrace in ~/.cargo/bin, so `git terrace` works anywhere
git terrace   # serves this checkout on 127.0.0.1 (make preview rebuilds and restarts it)
```

Run `make setup` once per machine. It keeps Spotlight, Windows Search and tracker or baloo out
of your checkout, `target/` and the bench cache, because rescans cost CPU and skew `make bench`,
which refuses to run until the exclusion is in place.

If the build fails on a clean checkout, that is a bug worth reporting.

Builds from inside the checkout read `.cargo/config.toml`, which runs the allocator with one
arena. `cargo install --git` builds elsewhere, misses it and keeps a few MB more while idle.

## Rules

- Read [AGENTS.md](AGENTS.md) for the code conventions. It is short. `make check` enforces
  most of them.
- gitterrace stays read-only. It never writes to the repository it shows. The only route that
  runs anything is the actions route, and it stays behind its token and its checks.
- Docs are generated from the code wherever they can be. Change the source (a registry, the
  CLI definition, a script header), then run `make docs-gen`. Never edit a generated block by
  hand.
- Every bug fix comes with a guard: a test or a check that fails on the bug and passes
  without it.
- Keep a change to one concern, and fix broken code you touch.

## Lint

`make lint` runs these, one pass or fail each, and `make check` includes it. Install any tool
it reports as missing.

- typos (`_typos.toml`): misspellings in code and prose
- vale (`.vale.ini`, `styles/gitterrace`): American spelling, no praise or history words, rules stated outright
- gitleaks (`.gitleaks.toml`): committed secrets
- markdownlint-cli2 (`.markdownlint-cli2.jsonc`): Markdown structure
- shellcheck and shfmt: `scripts/*.sh`
- ruff (`ruff.toml`): `scripts/*.py`
- cargo-machete: unused dependencies
- ast-grep (`sgconfig.yml`, `scripts/ast-grep/`): structural rules from AGENTS.md (gix only in
  gitterrace-git, no git subprocess, no `use super::` outside tests), each with a test

## Commits

Conventional Commits, `type(scope): subject`, with the subject under 72 characters. The
scope is the crate name without `gitterrace-`.

## Before a pull request

The local gate is the one that counts. The hosted workflows run a subset of it.

- `make check` always
- `make browser-check BIN=target/debug/git-terrace` if you changed templates, CSS or the scripts in
  `static/`. It drives real browsers (Playwright), so it is not part of `make check`. See
  `scripts/browser-check.mjs` for the checks and flags
- `make bench` if you changed how pages, the index or the server do their work (see
  [benchmarks](docs/benchmarks.md))
- Windows: CI builds and tests `x86_64-pc-windows-msvc` and smoke-tests the binary on a Windows
  runner. To run it locally on macOS or Linux, use `scripts/wine-run.sh`
- Linux: to check it locally in Docker, `scripts/linux-run.sh` builds and smoke-tests the binary
  (a few minutes), and `scripts/linux-run.sh --full` runs `make check` as the ubuntu CI job does
- macOS: `scripts/macos-run.sh` installs the binary and smoke-tests it with an empty HOME and a
  minimal PATH, so a dependency that only exists on your machine fails. `scripts/macos-run.sh --full` runs `make check`
  under the same clean HOME
- The three scripts exit 2 on a bad argument, and each has a self-test, part of `make check`,
  for its argument parsing

## Releasing (maintainers)

gitterrace.dev is generated into `site/` and shipped from a maintainer's machine with
`make deploy`. There is no deploy workflow and no Cloudflare token in the repository.

1. Bump `SECURITY_TXT_EXPIRES` in `xtask/src/site.rs`, `make docs-gen`, commit.
2. On the release candidate commit, in a clean clone on a quiet machine, `make bench LARGE=1`
   and commit the results. This is the only published run per release, never one in between.
3. `make changelog VERSION=x.y.z` writes the release's CHANGELOG.md section from the commits
   (git-cliff 2.14.2, `cliff.toml`), commit it. Never edit a released section by hand.
4. Tag and push. The release workflow builds the binaries, the installer and the formula.
5. `make deploy` (it refuses until the tag, repository and tap answer), then `make domains`.
6. Refresh the demo from [gitterrace/demo](https://github.com/gitterrace/demo): `make deploy`
   there.

## Security

Please do not report a vulnerability in a public issue. See [SECURITY.md](SECURITY.md).
