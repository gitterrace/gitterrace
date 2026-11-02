# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.1.0] - Unreleased

First public release.

- A local git browser: code, commits, blame, branches, worktrees, stashes and live
  updates, offline and read-only.
- Insights over history, cached in `.git/gitterrace/`. Pages read the cache from disk, so a
  server holds a few tens of MB however long the history is.
- JSON API (`/api/v1`), a read-only MCP server and a year-in-review card
  (`git terrace wrapped`).
- Optional user actions through `actions.toml`.
- Static export of a repository site.
- Every repository is served under `/r/<name>/`, a name kept across restarts, so a link stays
  valid when other repositories join. The repository switcher shows whenever the server
  holds more than one.
- Public mode (`--public`, `server.public`): a read-only viewer of committed history that
  is safe to serve beyond loopback, with `server.hosts` limiting which Host names answer
  and a 30 second cap per request.
- `git terrace open` and `install-handler` for `gitterrace://` links. Opening joins a
  running server and adds the repository to it. A second `git terrace` on the same
  address does the same instead of failing to bind. `git terrace remove` takes one off again.
  `install-handler` marks the app it writes and updates a marked app in place, and says
  when the program path changed. An app from an older build has no mark: remove
  `~/Applications/gitterrace-open.app` once, after that `install-handler` updates it.
  `git terrace open` hints when the handler runs a missing or another program.
- Windows builds.

### Fixed

- Mermaid diagrams keep their styles under the strict page policy, and pages carry no
  inline styles.
- Markdown styles stay inside rendered markdown.
- The last-commit column survives slow walks.
- Commit times are sane, one process writes the index at a time, and `.mailmap` and
  shallow roots are honored.
- Every file gitterrace writes goes through one no-follow atomic write.
- Error responses name no host paths, MCP counts match the web, and ETags follow the cache.
- Caches are bounded by bytes, and overview totals are computed once.
- Shell action scripts run as written.
- Small files are highlighted, and an empty selection reads as empty.
