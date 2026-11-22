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

The size metric is `git-terrace insights languages --json .` (`total_lines`), the
numbers of the Languages page for the HEAD tree. `make size` prints it.

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
- ast-grep (`sgconfig.yml`, `scripts/ast-grep/`): the structural rules from
  AGENTS.md, each with a test
- the shared house checks (personal data, own commit ids, internal references,
  the theme copy, the commit message hook), plus the repository's own: pinned
  actions, CSS scope, the site's content security policy, test configuration
- tsc and pyright: types for the scripts
- actionlint: the workflows
- the shared root check (`[root] allow` in `schwarzwald/config/lint.toml`):
  new files go under `scripts/`, `tests/`, `docs/`, `deploy/` or
  `schwarzwald/config/`

## Testing

Each layer owns one kind of claim, so a behavior is asserted once, in the
cheapest layer that can see it. A test that repeats a claim another layer owns
is a cut, not coverage.

| Layer | Where | Owns |
| --- | --- | --- |
| Unit | `mod tests` in the file | one function's logic: edge cases, error paths, tables of inputs |
| Integration | `crates/*/tests/` | crate-boundary behavior over a fixture repo: one behavior per file |
| Oracle | `crates/gitterrace-index/tests/`, `equivalence*` | the cache agrees with a direct git read, on hostile and large inputs |
| Golden | `crates/gitterrace-web/tests/golden/` | rendered markup, byte for byte: the place that owns HTML shape |
| Conventions | `crates/gitterrace-testkit/tests/conventions.rs`, `crates/gitterrace-web/tests/it/conventions.rs` | tree-wide rules (links, limits, plurals, routes) as one scan each |
| Browser | `scripts/verify/browser/` | what only a real browser sees: layout, focus, scripts, overlap |
| Bench | `scripts/bench/`, `make speed-check` | time and memory budgets, never correctness |

- A page's markup is asserted by the golden file, so an integration test does
  not grep the same HTML. It asserts the number or the link the page computes.
- Logic reached through a page is tested at the unit layer, and the page test
  checks only the wiring.

- Unit tests sit in one `#[cfg(test)] mod tests` at the bottom of the file they
  test, or in a `tests.rs` file declared under `#[cfg(test)]`. Platform or topic
  variants nest inside it (`#[cfg(unix)] mod unix`), never as a second top-level
  test module.
- Test-only helpers are gated with `#[cfg(test)]`, not left in production code.
- No section-divider banners in test code, blank lines separate groups.
- Integration tests live in each crate's `tests/`, one behavior per file.
- Two tests hitting the same branch with trivially different inputs become one
  table-driven test. A test that cannot fail is deleted.
- A test-only change that merges or drops tests carries its proof, the
  `make coverage-check` run described under
  [Before a pull request](#before-a-pull-request).

`scripts/lint/test-layout-check.py` enforces the first and third rules, with a
red and a green case in its self test.

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

## Pushing main

Main stays green on the hosted runners, so what is pushed has run CI first.
`make act-check` runs `ci.yml`'s push jobs through `act` (needs Docker) on a
clean clone of main and records the pass in `.git/act-pass/<sha>`.
`scripts/hooks/pre-push` refuses to push main at a commit without that record
or a `make release-gate` one. Run it after the last land, never before it:
the record names one commit.

## Releasing (maintainers)

gitterrace.dev ships from a maintainer's machine with `make deploy`, with no
deploy workflow or token in the repository. It refuses a dirty tree and builds
the site from HEAD into `target/site`. `make site-preview` serves the same site
on http://127.0.0.1:4343.

1. For the first release, set `Effective:` in `docs/legal/privacy.md` to the
   launch date. After that it moves only when the policy's content does.
   security.txt's `Expires` needs no step: `xtask/src/security_expires.rs`
   derives it from the newest changelog release plus a fixed span, and a test
   fails if it has passed or is over a year out.
2. On the release candidate commit, in a clean clone on a quiet machine,
   `make bench LARGE=1` and commit the results.
3. `make release-gate VERSION=x.y.z` on main: it removes any old x.y.z
   section, writes it with `make changelog` and commits it, so a hand edit or
   an old date never ships. It then runs both changelog checks with the tag on
   that commit, the gate on macOS, the smoke test, and `ci.yml` with
   `full=true` on the hosted runners for that exact commit (pushed to a temporary branch,
   deleted after): some races show only on x86_64 runners, never in `act` on
   an arm64 Mac. `ACT=1` adds the Ubuntu jobs through `act`. On pass it pushes
   main and the tag in one atomic push. Never tag or push a tag by hand:
   `scripts/hooks/pre-push` refuses a tag without a pass record naming its CI
   run, a tag apart from a main at the same commit, and a main that edits a
   released section apart from its tag. To re-tag an unpublished release,
   `make release-gate VERSION=x.y.z RETAG=1`.
4. The release workflow builds the binaries, the installer and the formula.
5. `make deploy` (it refuses until the tag, repository and tap answer, and a site
   that fails a site guard), then `make domains`.
6. Refresh the live demo: `make demo demo-deploy` exports the pinned jq and
   ripgrep and ships them to demo.gitterrace.dev.

## Security

Report vulnerabilities privately, never in a public issue: see
[SECURITY.md](SECURITY.md).
