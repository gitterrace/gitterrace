<h1 align="center">
<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/readme-header-dark.svg">
  <img alt="gitterrace" src="assets/brand/readme-header-light.svg" width="600">
</picture>
</h1>

<p align="center"><b>Your repository's history, step by step. A GitHub-style view of any git repo, offline, in one binary. Run <code>git terrace</code>.</b></p>

gitterrace serves any git repository in your browser: code, commits,
branches, blame and rendered markdown (with mermaid), plus insights that go
past what GitHub and GitLab show. It is read-only and local, with no database,
no accounts and no network.

```bash
git terrace                       # the repo you are in, on 127.0.0.1:4242
git terrace ~/src/app ~/src/lib   # several repos on one port
git terrace ~/src                 # every repo in a folder, two levels deep
```

## What you get

- **Browse.** Tree, files with highlighting, raw, blame, commits with
  diffs, branches and tags, and a view of uncommitted changes. Line links
  (`#L12-L20`) and permalinks.
- **Find.** `t` opens a file finder, `/` searches code and commits, and `?`
  lists every key.
- **Markdown.** GitHub-flavoured, with alerts, anchors, relative images and
  mermaid diagrams, sanitised.
- **Insights.** Activity, hot files, commit types by scope, lines of code by
  language over time, and authors. One filter drives every page: type, scope,
  author, path and period, as removable chips. Click an author to see their
  work.
- **Several repos.** One port, a repo index at `/`, and a switcher in the
  header. Each repo lives at `/r/<name>/`.
- **Fast.** Pages render in a few milliseconds on a 14,000-commit repo.
  Responses are compressed, and huge diffs load on demand.

## Commands

| Command | Does |
|---|---|
| `git terrace [paths]` | Serve in the browser (`--addr` to change the address) |
| `git terrace export <out> [path]` | Write a static copy of the site (`--commits`, `--max-file-kb`) |
| `git terrace wrapped [path]` | Write a year-in-review card (`--year`, `--layout`, `--theme`, `--png`) |
| `git terrace mcp [path]` | Run a read-only MCP server over stdio, for agents |

`gitterrace` is the same binary under its own name. A JSON API lives at
`/api/v1`; see [the API reference](docs/api.md).

## Safe by default

It binds to `127.0.0.1`. It refuses requests whose Host is not local (DNS
rebinding) and requests from other websites, and it sends a strict Content
Security Policy. It never changes your history, refs or working tree; its only
write is a cache in `.git/gitterrace/`, built once in the background and safe
to delete.

## Install

```bash
brew install timorunge/tap/gitterrace
```

Or build it: `cargo install --path crates/gitterrace`. MIT licensed.

More: [architecture](docs/architecture.md), [design](docs/design.md).
