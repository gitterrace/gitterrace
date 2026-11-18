# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

<!-- BEGIN GENERATED: unreleased -->
<!-- END GENERATED: unreleased -->

## [0.1.1] - 2026-11-18

### Added

- export: a footer like the header, and a not-found page
- web: a binary file names its format, size and a download link
- web: a folder page offers a code search scoped to that folder
- web: render Jupyter notebooks on the blob page
- diff: fold generated files in diffs and search
- web: open a changed file from the working tree in the blob view
- web: show how far the branch is ahead of or behind its upstream
- web: render CSV and TSV blobs as a table
- web: list the entries of zip, jar, tar and tar.gz blobs
- web: render the first sheet of xlsx and ods blobs as a table

### Fixed

- export: drop a font preload whole, however it is wrapped
- index: make the cache folder again when it is gone before a write
- web: diff and code rows keep the code size, so the gutter band is even
- export: leave links the export cannot serve as text
- export: the repository switcher works between exported repositories
- web: light the followed symbol, not the one below it
- web: land the about card flush at the top of the window
- web: line up the working tree's file links in one column
- alloc: turn transparent huge pages off on Linux
- index: run a segment merge after the update releases its lock
- web: meet target size, link and avatar contrast rules
- ci: shellcheck composite action run blocks
- deps: ignore the chrono unmaintained advisory

### Performance

- web: page the blame view in rows of 2000
- git: bound the past versions one blame reads
- git: keep 8-byte id prefixes in the public reachable set
- api: read the web pages' derived tables for insights
- alloc: enable thread caches inside index workers only
- web: bound the blame warm-up by time so an idle server releases
- web: keep memoized pages deflated
- alloc: keep small objects in one-page slabs on aarch64 Linux
- web: build the index in a child process
- git: walk only the blamed file's history before a blame
- git: skip the blame horizon walk when the versions fit
- git: walk the blame horizon beside the counting pass
- export: stage pages without a sync per file

## [0.1.0] - 2026-11-14

### Added

- Browse a local repository offline: code, commits, the commit graph, blame,
  branches, tags, worktrees and stashes, served on `127.0.0.1`.
- Insights from an incremental cache in `.git/gitterrace/`: pulse, hotspots,
  ownership, patterns, languages, compare and file history, with one filter.
- Search code and commits, a file finder, keyboard shortcuts and permalinks
  that survive clones and renames (`gitterrace://` and `/go`).
- Markdown with alerts, highlighting for about 200 languages and mermaid.
- Light and dark themes that follow the system, with live updates while the
  working tree changes.
- Several repositories on one port under `/r/NAME`, joined by running
  `git terrace` again in another repository.
- A JSON API (`/api/v1`) and a read-only MCP server for agents.
- `git terrace export`: a static copy of the site, which follows the public
  rule.
- User actions in `actions.toml`: run a program on a selection or send it to a
  tracker, notes app or mail draft. Off until configured.
- A public mode for hosting, a Docker image, and installers for Linux, macOS
  and Windows (shell, PowerShell, Homebrew).
