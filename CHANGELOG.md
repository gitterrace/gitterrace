# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

<!-- BEGIN GENERATED: unreleased -->
## [Unreleased]

### Added

- site: refuse a docs page that does not open with its title and a lead
- site: deploy-check runs the site guards on the built site
- site: derive security.txt Expires from the newest release
- api: serve the languages and ownership insights over HTTP and MCP
- scripts: take size-report's total and languages from gitterrace
- web: show the running version at the foot of the actions menu
- web: keep the view when switching repository
- web: fit insights charts to the selected range
- api: serve every insight over MCP from the registry's one list
- cli: print any insight as JSON through the registry's handler
- cli: run the server as a systemd or launchd service
- site: say what the tool is for in the tagline
- site: lead the landing page with install, trust and the hand-off
- harness: run the browser checks in Firefox
- review: checkpoints of uncommitted work and viewed marks
- actions: derive an owner session value and cookie name from the secret
- cli: open the browser through a one-time launch link
- review: a review basket of findings, listed with where their code is now
- web: add a diff toolbar with find, viewed progress and hide viewed
- api: serve the card of one file or directory
- api: rank the changed files of a review by risk
- web: show a card for a file or folder on the blob, tree and history pages
- web: style the not-found and error pages and say why a repository failed
- web: give the commit page two header rows and a readable message
- insights: explore which files change together
- web: checkpoint button, since tab and server-kept viewed marks
- web: add findings to the review basket and send them all
- web: review order panel on the working tree and compare pages
- insights: show which knowledge rests on people who left

### Changed

- api: one ParamDoc for endpoint, tool and prompt parameters
- api: apply request defaults once in ApiState and share diff totals
- api: one helper each for refused, JSON and joined tool results
- config: one macro table for the feature enum, keys and file values
- config: settings fields carry the value Control, not a second Input enum
- actions: one home lookup and expand_tilde in types, one kind_name
- actions: share the yes default, label check, id rule, flat line and line
  fragment
- export: one link scanner for the followed links and the rewrite
- open: one HTTP exchange for the probes and the add request, one is_wsl
- bin: repos_of and server_addr, export arm passes its fields
- web: one build entry, one bind prelude, one ignore_for
- web: one poll loop for the actions and config reloaders
- web: one JSON body and no-store helper for the write routes
- web: Slots and DirtyMemo types for the shared semaphores and dirty answer
- web: one add path for the repository table, Served base and href
- web: Empty, Seg, CommitRow and PageHead render from their own template
- web: Chrome::from_cfg and Default so the bare bar states only what differs
- web: one option table for the insights links and query pairs
- web: insights pages share scope, head and section helpers
- web: commit type matrix counts its cells once
- web: compare shares its unreadable state and signed numbers
- web: highlight merges spans and looks syntax tokens up in one place
- web: settings replies build their error in one place
- web: stats tabs are a plain list, one file stat row builder
- web: diff file sections bind the generated check once
- web: wrap the page head tab row template
- xtask: the wordmark check uses the theme's contrast
- xtask: one footer_of for the site tests
- xtask: one write path and one fill for generated output
- xtask: one flag list and one positionals for the command docs
- scripts: one parse_args, toolchain_pin and fail for the run scripts
- scripts: one commitFiles and static imports for the browser libs
- web: keep the non-language extensions in one split string
- web: keep id and keyword lists as split strings
- search: share the haystack writer and the regex size limit
- charts: share the plot geometry and the 1-2-5 step walk
- search: clip the query with floor_char_boundary
- web: static scripts read the base and build elements through gtNav
- web: log and commit types share one pager template
- bench: keep the cold sample count in one place
- web: reuse the base and el helper from nav.js in actions, send and settings
- git: one newest-first rev walk helper
- git: one commit summary and subject builder
- git: one counts struct for the numstat memo
- git: share the blob read, the plain path check and the nudge loop entry
- git: one no-follow create shared with the index
- index: one canonical topological sort shared with git
- index: one warn helper holds the stderr exemption
- index: the disk source folds its aggregates through the shared folds
- index: drop the segment merge module nothing ships
- index: one sums accumulation for the column item and the overflow checked adds
- index: one identity-pair mapping step for the oracle and the fresh walk
- web: one repo_router, not a wrapper over its tasks variant
- web: frequency section without a redundant block
- web: fold the action menu builder into its only caller
- web: inline the far page's one-use message builder
- web: inline the single-use list filter helper
- index: drop the oracle's clock-injecting reference_time twin
- git: drop the changed_paths forwarder
- xtask: one helper prints findings and exits
- xtask: fold the wrap_line forwarder into fill
- scripts: drop the docker check's unset port and jobs knobs
- scripts: reuse empty_commit in the land-check self-test and drop a redundant
  grep
- web: share the extension sniff, subjects read, scoped guard, local_state and
  sha anchor
- web: build the warm-up router once and trim one-use plumbing
- web: share the settings file read, the browser refusal and JSON replies
- web: render media markup from templates
- api: drop test hooks, one default diff cap, no blob line vec
- git: one list of public refs, one worker count, no dead reads
- xtask: drop unwrap in prose file walk
- scripts: read size-report's languages from the insights command
- browser: serve the built site from one helper
- browser: share the per-browser loop, the luminance and the stats page list
- dev: one usage branch in the run scripts' argument parser
- web: move the screen fixes to screens.css and wrap the new lines
- xtask: split the docs generator into one module per page

### Fixed

- site: every docs page opens with its title and a lead
- web: wait for the repo id before comparing whole stats pages
- lint: the deps check's tomllib import passes pyright under an older python3
- web: keep the Find box above the Viewed count in either load order
- verify: coverage-check reads lines hit by a race as unstable
- verify: coverage-check names the unstable lines it forgave
- web: align the Viewed toggle in a fixed slot and put Find and the count on one
  row
- web: give a Markdown file the same pinned page head as every file
- web: hold a reader at the top of a live page at scroll 0
- web: offer search on the settings page
- web: do not throttle an index build for the load its own workers make
- web: keep a corrupt registry file instead of resetting it
- git: refuse a committed-only rev with a stray character after its steps
- web: keep only checkbox inputs in rendered markdown
- scripts: write the preview log under target, not a fixed /tmp path
- dev: find busy clones through /proc where lsof is missing
- dev: sweep after a plan landing too
- dev: keep a fresh work clone the sweep would take for landed
- index: refuse tables built over a failed read, look people up without indexing
- build: point the harness link check at the shared kit's current path
- browser: serve the working tree's assets through --from-disk
- browser: wait on conditions instead of fixed sleeps
- browser: keep the wait helpers inside the line and type gates
- index: bound every stored count by the bytes left before sizing a buffer
- sweep: trash landed clones and keep any younger than 30 minutes
- web: honour the shortcut switch in every printable-key handler
- web: require an owner cookie for writes and serve no token in pages
- git: count every worktree tab from the rows its table lists
- web: put the file list before the sidebar and make phone targets 44px
- web: tighten the page head, order headings and share the header brand
- web: touch targets, focus rings inside scrollers and sizes on the tokens
- web: the page key handlers for b, dot and slash read the shortcuts switch
- web: close the page head 16px below the tagline again
- site: refuse a partial raw shot set before replacing the screenshots
- deps: keep the unmaintained advisory scope on the whole graph

### Performance

- web: serve every stylesheet minified
- web: bundle the shared stylesheets into two requests
- web: preload only the faces used above the fold
- web: load the markdown rules only on pages that render markdown
- site: drop two stylesheet rules no page matches
- web: read the index once for a filtered log page
- web: skip the patterns reads when the period counts no commit
- index: borrow message text in the search scan and encode an item once
- index: move message items into the merged column instead of cloning
- export: move the marked page into the footer step instead of cloning it
- git: drop repeated stats and scans from the watch, nudge and move paths
- index: dedupe per commit in linear time and stop leaking tip errors
- config: build the key table once per parse and validate
- xtask: read the cargo metadata once per run
- git: keep byte budget free for the items near the head of the stream
- web: cut one element per file from diff headers and file rows

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
