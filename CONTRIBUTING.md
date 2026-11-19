# Contributing

Thanks for taking a look. Issues and pull requests are welcome. Agents write
and review most of the code and a human decides what goes in
([why](docs/why.md)). The rules apply to everyone, and most are checks that
run.

A pull request is offered under the MIT license ([LICENSE](LICENSE)).

## Setup

You need git. `make deps` installs the rest at the versions CI uses: the Rust
toolchain `rust-toolchain.toml` pins, Python 3.11 or newer, node and the
linters `make check` runs. On macOS it uses Homebrew, so install that first.
`make deps BROWSERS=1` also fetches the browsers the browser checks drive.

```sh
git clone https://github.com/gitterrace/gitterrace && cd gitterrace
make deps
make build
make install  # git-terrace into ~/.cargo/bin, so `git terrace` works
git terrace   # serves this checkout on 127.0.0.1
```

`make preview` rebuilds and restarts the served checkout. Run `make setup` once
per machine: it keeps desktop search indexers out of the checkout and caches,
and `make bench` refuses to run without it. A build failure on a clean checkout
is a bug worth reporting. In-checkout builds read `.cargo/config.toml` (one
allocator arena), so `cargo install --git` idles a few MB higher.

## Rules

- Read [AGENTS.md](AGENTS.md) for the code conventions. `make check` enforces
  most of them.
- gitterrace stays read-only. Only the actions route runs anything, behind its
  token and checks.
- Docs are generated from the code wherever they can be. Change the source,
  then run `make docs-gen`. Never edit a generated block by hand.
- Every bug fix comes with a guard: a test or a check that fails on the bug and
  passes without it.
- Keep a change to one concern, and fix broken code you touch.

## Lint

`make lint` runs these, one pass or fail each, and `make check` includes it.
Install any tool it reports as missing.

- typos (`_typos.toml`): misspellings in code and prose
- vale (`.vale.ini`, `schwarzwald/styles/`): American spelling, no praise
  or history words, rules stated outright
- gitleaks (`.gitleaks.toml`): committed secrets
- markdownlint-cli2 (`.markdownlint-cli2.jsonc`): Markdown structure
- shellcheck and shfmt: the shell scripts under `scripts/`
- ruff (`ruff.toml`): the Python scripts under `scripts/`
- cargo-machete: unused dependencies
- ast-grep (`sgconfig.yml`, `scripts/ast-grep/`): structural rules from
  AGENTS.md (gix only in gitterrace-git, no git subprocess, no `use super::`
  outside tests), each with a test
- the shared house checks (personal data, own commit ids, internal references,
  the theme copy, the commit message hook), plus the repository's own: pinned
  actions, CSS scope, the site's content security policy, test configuration
- tsc and pyright: types for the scripts
- actionlint: the workflows
- the shared root check (`[root] allow` in `schwarzwald/config/lint.toml`):
  new files go under `scripts/`, `tests/`, `docs/`, `deploy/` or
  `schwarzwald/config/`

## Commits

Conventional Commits, `type(scope): subject`, with the subject under 72
characters. The scope is the crate name without `gitterrace-`. The body says
what changed, why, and how it is tested. Run
`git config core.hooksPath scripts/hooks` once so the message rules run on
every commit.

A change lands with
`bash vendor/schwarzwald/kit/tools/land.sh <repo> <clone> <sha>`, one commit at a
time on main's tip. The landing checkout (marked with
`git config --local landing.target "$(git rev-parse --show-toplevel)"`) refuses
a plain `git commit`, so commit in a clone and land from it.

## Before a pull request

The local gate is the one that counts. Hosted CI runs the Linux `make check`,
cargo deny, the MSRV check and, when a Docker path changed, the image. macOS
and Windows run weekly, on demand and for every tag.

- `make check` always
- `make browser-check BIN=target/debug/git-terrace` if you changed templates,
  CSS or `static/` (Playwright). Run `make browser-deps` once per clone first.
  Flags are in `scripts/verify/browser/browser-check.mjs`
- `make bench` if you changed how pages, the index or the server do their work
  (see [benchmarks](docs/benchmarks.md))
- Other systems locally: `scripts/dev/wine-run.sh` (Windows),
  `scripts/dev/linux-run.sh` (Docker) and `scripts/dev/macos-run.sh` (clean
  HOME and PATH) build and smoke-test the binary, and `linux-run.sh --full` or
  `macos-run.sh --full` runs `make check` there. Those two exit 2 on a bad
  argument.
- `make coverage-check BASE=<parent> REV=<commit>` if the change only moves or
  merges tests. It fails when a product line covered at `BASE` is uncovered at
  `REV`, a crate's branch percentage drops or a caught mutant is missed. Set
  `MUTANTS_PKG` and `MUTANTS_FILES` (src globs) to add cargo-mutants on the
  files whose tests moved. Install once with
  `cargo install cargo-llvm-cov cargo-mutants` and
  `rustup toolchain install nightly --component llvm-tools-preview`: branch
  coverage is nightly-only on the pinned compiler, so both sides run on the
  same `NIGHTLY` (default `nightly`). Not part of `make check`.

## Releasing (maintainers)

gitterrace.dev ships from a maintainer's machine with `make deploy`, with no
deploy workflow or token in the repository. It refuses a dirty tree and builds
the site from HEAD into `target/site`. `make site-preview` serves the same site
on http://127.0.0.1:4343.

1. Bump `SECURITY_TXT_EXPIRES` in `xtask/src/site.rs`, commit. For the first
   release, set `Effective:` in `docs/legal/privacy.md` to the launch date.
   After that it moves only when the policy's content does.
2. On the release candidate commit, in a clean clone on a quiet machine,
   `make bench LARGE=1` and commit the results.
3. `make changelog VERSION=x.y.z` writes the CHANGELOG.md section from the
   commits, commit it. Never edit a released section by hand.
4. Tag and push. The release workflow builds the binaries, the installer and the
   formula.
5. `make deploy` (it refuses until the tag, repository and tap answer), then
   `make domains`.
6. Refresh the live demo: `make demo demo-deploy` exports the pinned jq and
   ripgrep and ships them to demo.gitterrace.dev.

## Security

Report vulnerabilities privately, never in a public issue: see
[SECURITY.md](SECURITY.md).
