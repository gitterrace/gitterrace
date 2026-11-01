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
Python 3. The site and logo checks use `uv` if present.

```sh
git clone https://github.com/gitterrace/gitterrace && cd gitterrace
make build
make run      # serves this checkout on 127.0.0.1
```

If the build fails on a clean checkout, that is a bug worth reporting.

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

## Security

Please do not report a vulnerability in a public issue. See [SECURITY.md](SECURITY.md).
