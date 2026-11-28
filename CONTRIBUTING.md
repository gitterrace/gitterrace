# Contributing

Thanks for taking a look. Issues and pull requests are welcome. Agents write
and review most of the code and a human decides what goes in
([why](docs/why.md)). The rules apply to everyone, and most are checks that
run.

A pull request is offered under the MIT license ([LICENSE](LICENSE)).

## Setup

You need git. `make deps` installs the rest at the versions CI uses: the Rust
toolchain `rust-toolchain.toml` pins, Python, node and the
linters `make check` runs. On macOS it uses Homebrew, so install that first.
`make deps BROWSERS=1` also fetches the browsers the browser checks drive, all
of them run by default. A `make lint` step fails if the harness can launch one
that the install lines skip.

```sh
git clone https://github.com/gitterrace/gitterrace && cd gitterrace
make deps
make build
make install  # the binary into ~/.cargo/bin, so `git terrace` works
git terrace   # serves this checkout on 127.0.0.1
```

`make preview` rebuilds and restarts the served checkout. Run `make setup` once
per machine: it keeps desktop search indexers out of the checkout and caches,
and `make bench` refuses to run without it. A build failure on a clean checkout
is a bug worth reporting. In-checkout builds read `.cargo/config.toml` (one
allocator arena), so `cargo install --git` idles a few MB higher.

The size metric is `git terrace insights languages .` (`total_lines`), the
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

`make lint` runs every linter and shared check, one pass or fail each, and
`make check` includes it. The list lives in
`vendor/schwarzwald/kit/lint/run.sh` and `schwarzwald/config/lint.toml`, so it
cannot go stale here. Install any tool it reports as missing. Prose is held to
`.vale.ini` (American spelling, no praise or history words, rules stated
outright), and the structural code rules in `scripts/ast-grep/` each carry a
test.

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

A change that alters what a setting means (not only what it does) carries a
`Changed-behavior: <one sentence>` trailer. `cliff.toml` renders each one under
"Changed behavior" in that release's changelog section, which refuses hand
edits. `commits-check` prints a `HINT` for a commit that edits a key
description in the config registry or `docs/configuration.md` without it.

A change lands with
`bash vendor/schwarzwald/kit/tools/land.sh <repo> <clone> <sha>`, one commit at a
time on main's tip. The landing checkout (marked with
`git config --local landing.target "$(git rev-parse --show-toplevel)"`) refuses
a plain `git commit`, so commit in a clone and land from it.

## Before a pull request

The local gate is the one that counts. Hosted CI runs the Linux `make check`,
the browser checks (`make browser-check` on a release build, every case in
chromium, webkit and firefox, uploading `browser-check.log` when red), cargo
deny, the MSRV check and, when a Docker path changed, the image. The release
gate runs the browser checks locally too, skipped with a loud note only when
`make browser-deps` has not run. `scripts/lint/browser-gate-check.py` fails when the
`browser` job is missing or narrows the run. macOS
and Windows run weekly, on demand and for every tag. The macOS gate is two
parallel jobs (static analysis and docs, then tests and lint), so each stays
well inside its timeout. `scripts/verify/ci-budget-check.py` is a step of
`make release-gate` (its self-test is in `make lint`), skipped with a note
without `gh`. It fails when a job's median over
recent successful main runs passes 75% of its `timeout-minutes`: split the
job, never raise the timeout.

- `make check` always
- `make browser-check BIN=target/debug/git-terrace` if you changed templates,
  CSS or `static/` (Playwright). Run `make browser-deps` once per clone first.
  Flags are in `scripts/verify/browser/browser-check.mjs`.
  A same-origin 4xx or 5xx fails a case unless its `EXPECT` entry lists it.
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

Both `make act-check` and the hook also ask the host (`gh run list`) for the hosted
CI result of the remote's current main. A failed job (named in the message) or a
run still in progress refuses the push, so a hosted-only job such as the
reproducible build cannot stay red unnoticed. Without `gh` or a network they
only warn. The harness sets `TEST_ALLOW_PENDING_CI=1` to pass a running run.
The push that fixes a red main sets `TEST_ALLOW_RED_CI=<run id>`,
comma-separated when the commit has more than one failed run (its push and a
manual full run): it passes only when the failed runs are exactly the ones named
(a stale or partial list matches nothing), prints a loud line naming each run
and its failed jobs, and every other rule still runs. Do not use
`git push --no-verify`, which skips them all.

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
   that commit, the gate on macOS, the smoke test, the benchmark regression
   check (`bench-report.py --regress`, no override: the newest run must not be
   worse than the previous release's on the same machine, outside the noise
   band), and `ci.yml` with
   `full=true` on the hosted runners for that exact commit (pushed to a temporary branch,
   deleted after): some races show only on x86_64 runners, never in `act` on
   an arm64 Mac. `ACT=1` adds the Ubuntu jobs through `act`. On pass it pushes
   main and the tag in one atomic push. Never tag or push a tag by hand:
   `scripts/hooks/pre-push` refuses a tag without a pass record naming its CI
   run, a tag apart from a main at the same commit, and a main that edits a
   released section apart from its tag. To re-tag an unpublished release,
   `make release-gate VERSION=x.y.z RETAG=1`.

   The release date comes from one place: the committer date, in UTC, of the
   commit that set the version (`vendor/schwarzwald/kit/tools/release-date.sh`).
   `make changelog` writes it, the tagged-section check fails when the
   changelog says otherwise, and the release gate exports `SOURCE_DATE_EPOCH`
   from it, so a gate that crosses midnight changes nothing. Commit the version
   bump before running the gate. From 0.2.3 on, a release is dated by its bump
   commit's UTC date. Earlier sections keep the dates they were published with
   (`dates_since` in `schwarzwald/config/lint.toml`).
4. The release workflow builds the binaries, the installer and the formula.
5. `make deploy` (it refuses until the tag, repository and tap answer, and a site
   that fails a site guard), then `make domains`.
6. Refresh the live demo: `make demo demo-deploy` exports the pinned jq and
   ripgrep and ships them to demo.gitterrace.dev.
7. `make reach` prints adoption: release downloads per os and arch, tap clones
   and visitors, each with the change against a week earlier. It reads the
   code host through `gh`, writes a snapshot under
   `~/.local/share/schwarzwald/reach` and is never part of the gate.
   `schwarzwald/config/reach.toml` holds the repository, the tap, the
   downloads our own CI causes per release (`ci_baseline`) and the asset
   globs.

## Security

Report vulnerabilities privately, never in a public issue: see
[SECURITY.md](SECURITY.md).
