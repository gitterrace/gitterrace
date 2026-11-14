# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

<!-- BEGIN GENERATED: unreleased -->
<!-- END GENERATED: unreleased -->

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
