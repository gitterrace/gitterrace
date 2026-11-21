# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

<!-- BEGIN GENERATED: unreleased -->
<!-- END GENERATED: unreleased -->

## [0.2.0] - 2026-11-21

### Added

- index: CPU seconds and worker busy share per index step
- web: a repository reports queued, indexing, warming, ready or failed
- web: POST /_repos answers 202 at once and opens the repository after
- release: gate every tag on hosted CI and push it atomically with main
- web: one warm-up registry, run in full before a repository is ready
- web: warm the hotspot periods and gate first pages on linux too
- web: answer a page the warm-up is rendering within 2 seconds
- index: resize the index workers of a running build from the server
- index: drop the stored chain file, cache format 2
- web: fold generated files in the working tree diff
- web: budget the index workers by machine load
- web: extend the languages chain from a saved state
- actions: add starter entries for coding agent CLIs
- web: split the working tree page into Staged, Unstaged and All
- web: answer an added repository at once and open it in the background
- compare: three-dot range, fork link on branch logs and --ref flag
- api: compare and worktree_diff tools and a review_change prompt
- web: jump between diff files with ] and [, count folded generated files
- xtask: mark generated docs and check the openapi types
- web: mark diff files viewed with a progress count and a changed-since badge
- web: find in the whole diff on the server, folded files included
- web: a color-blind diff palette as display.diff_palette
- web: diff wrap, tab width and code font size as display settings
- web: show a font's family, styles and glyph count with a sample
- xtask: refuse a built page over its kind's byte budget

### Changed

- index: drop the unwired accelerator
- index: drop the test-only single rank read
- git: decode a read blob in one place
- index: fold the commit-size module into period
- bench: store result runs one line per metric
- lint: fold ci-check into gate-check
- lint: move the design-pointer rule into ast-grep
- lint: one table for the regex text lints
- bench: call the bench libs by module, drop the alias blocks
- xtask: one csp_hash for the site CSP and its check
- index: use rustc-hash for the statistic maps
- types: keep the small API shapes in one module
- api: one macro for the MCP argument structs
- Drop one-line docs that restate a private name and an unused constant
- index: forward the plain source methods through one macro
- verify: discover browser checks from the lib exports
- bench: read the corpus through one parser
- verify: one commitAll helper for the browser fixtures
- xtask: lay the block tag table out in rows
- web: share el and the toast and copy helpers between scripts
- web: fold the line delta and section jumps into their users
- web: build the filter's bucket range terms in one pass
- xtask: generate the theme, logos and tagline check in Rust
- index: fold author-day and path-author rows in one place
- index: drop the single-caller report helpers
- open: fold error tests into the open tests
- actions: build the three config errors the same way
- verify: one sweep helper for the browser, scheme and width loops
- verify: report browser verdicts through one reporter method
- xtask: one heading slug, one extension test and one attribute scan for the
  site
- index: share one Sums type between the feed and segment aggregates
- web: build both event streams with one function
- index: build rename edges in one place

### Fixed

- site: check the live CSP against every page it covers
- web: load the hover tip script on the repositories home
- index: create cache temp files as new inodes
- web: skip a clone whose .git holds no HEAD yet
- export: check the license blob size before reading it
- worktree: cap staged paths and hold the raw budget for worktree files
- worktree: open worktree files beneath the folder and bound the read
- web: run API reads and the home under the page job slots
- web: bound a workbook's grid before calamine builds it
- web: say per tab what an empty working tree tab means
- web: keep one index worker for a build on a saturated machine
- worktree: put the scope tabs in the page head
- worktree: count a staged rename as one listed file
- api: one log page default for HTTP and MCP
- testkit: oracle ready waits for the warm-up to finish
- scripts: act-check sets the runner user in its image, not in act's container
  options
- api: gate the rebuild flag hook to unix, where its only test runs
- land-check: the landing gate runs the scoped Windows clippy too
- verify: lint the Windows target without a linker and keep act's full log
- site: wrap the wordmark data inside the line width
- verify: the golden proof dump passes its test names after --
- release: keep the Unreleased block above a new release section

### Performance

- index: diff merge commits first in the index work queue
- land-check: run the oracle on its own only when its crate's suite did not
- land-check: run the tree conventions once per landing
- site: write the wordmark outline as relative path data
- site: minify the stylesheet the site serves
- web: size the punch card cells in the stylesheet, not on each rect

## [0.1.1] - 2026-11-19

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
- index: never make a deleted .git again for the cache
- web: a repository taken off while its router builds keeps nothing running
- web: stop index writes of a repository taken off
- web: one index child per repository at a time, so a removal stops them all

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
