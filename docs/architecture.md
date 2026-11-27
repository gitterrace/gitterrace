# Architecture

How the crates fit together, the decisions behind them, and where the cache,
routes and guards live.

<!-- BEGIN GENERATED: crates -->
```mermaid
flowchart TD
  gitterrace["gitterrace: the binary and CLI"]
  actions["actions: actions.toml and Send to"]
  api["api: JSON API and MCP"]
  config["config: config.toml"]
  git["git: all git reads, via gix"]
  index["index: insights cache"]
  types["types: shared types, each defined once"]
  web["web: server, pages, guard"]
  gitterrace --> web
  actions --> git
  api --> actions
  api --> index
  config --> types
  git --> types
  index --> config
  index --> git
  web --> api
```

<!-- END GENERATED: crates -->

Each box is a crate the binary ships, and an arrow means "uses" (implied arrows
left out). `testkit` is test-only. `schwarzwald` is the vendored shared kit:
theme, lints, hooks and guards.

The API crate's entry points are in its rustdoc. The naming rule for config
keys lives in `gitterrace_config::naming`.

## Decisions

- **The theme is a snapshot, not a design.** Colors, type, space and motion come
  only from `vendor/schwarzwald/kit/`, replaced as a whole, never edited.
  `xtask/src/theme.rs` writes the block in `tokens.css`.
  `make theme` fails on drift. Components use the role aliases, never a raw
  color, and charts are server-rendered SVG colored by class.
- **Read-only, pinned.** Every read takes a resolved commit id, so a commit
  landing mid-request cannot tear a page. Only the uncommitted view reads the
  worktree, and so does the review basket: an anchor is checked against it.
  The file view marks the anchored lines of the basket (`blob_marks.rs`, from the same
  anchors, drawn on the page and never kept in the highlight memo).
  Every findings surface (page, JSON API, MCP tool and review prompt) is
  gated on `worktree_enabled`, file-wide or per repository, and is refused on
  a public server. The repository and the worktree are never written: the
  only writes
  are the files of the cache folder, all through `cache.rs`. Of those, the index
  is disposable, while the user's files (see [Cache](#cache)) survive every
  reset (`USER_FILES` in `store.rs`).
- **Browsing never waits on the cache.** Only insights pages show "Indexing
  history" while the index builds. The overview shows no progress.
- **The index builds in a child process.** The server starts its own executable
  again to update the cache, then reads the segment set from disk. The build's
  peak (allocator metadata, worker stacks) exits with the child, so an idle
  server holds the same at 100k commits as at 2k
  (`gitterrace-web/src/app/child.rs`).
- **Assets are versioned by build.** Every `/static/` asset is linked as
  `/static/<name>?v=<build hash>` (`asset_url` in
  `gitterrace-web/src/routes/assets.rs`). At the current version it is
  immutable for a year, otherwise `no-cache`. Exports keep `/static/*` a year,
  and the site versions `/assets/*` by content sha.
- **One guard** (`gitterrace-web/src/guard.rs`): loopback Host, no cross-site
  requests, hashed inline script CSP.
- **Live.** A watcher pushes changes over `/events`, and config files reload on
  change, and a joining repository appears in open switchers. OS file events
  watch the git folder always and the working tree while a tab shows it. A burst
  of events is one cheap check. Dropped events (a loaded Mac) are caught by the
  `watch.rescan_secs` rescan. Without file events the server
  checks once a second.
- **Features switch off at the route.** Each `[features]` switch owns its routes
  (`gitterrace-web/src/features.rs`). One gate answers an off route with 404,
  and pages ask the same table, so no link is left behind. Off always wins.
- **Markdown renders as the hosts do.** Heading anchors are the plain slugs
  (`#getting-started`). Only an id the page itself uses gets a prefix.
- **Every repo under `/r/{name}/`**, also when it is the only one, resolved only
  through the repository table. `/` forwards to a lone repository and lists
  several. A link never moves when another repository joins.
  `/go/{repo-id}/{sha}` is the permalink and outlives names.
- **A name is assigned once:** the folder name, URL-safe and lowercase. A clash
  gets `-` and six hex digits of its path, and the first keeps the plain name.
  Names persist in `repos.json`, so a restart serves each where it was. Without
  a state folder the rule runs against the table alone.
- **A running server grows, it is never replaced.** `git terrace open` and a
  second `git terrace` add repositories through `POST /_repos`, and
  `git terrace remove` takes one off with `DELETE /_repos`. The gate (loopback
  only, browser headers refused, a 0600 token, a nonce check of the listener) is
  in [SECURITY.md](../SECURITY.md). A foreign listener gets no path: `open`
  starts its own server and `serve` binds as usual. A second `serve` opens every
  root before it adds any, then exits 0. Once adding starts, a refused add is an
  error, never a bind on the held address. `--public` never joins. Removal keeps
  the index and name, stops its refresher and drops it from open switchers. The
  last repository stays. No page offers remove, since a page has no token. An
  added repository gets what a startup one does, and one holding `actions.toml`
  is refused.
- **`gitterrace://` permalinks name what, not where.**
  `gitterrace://<repo-id>/<sha>/<path>` is the `/go` tuple without a host.
  `git terrace open` finds the clone among repositories served before
  (`$XDG_STATE_HOME/gitterrace/repos.json`, else `~/.local/state/gitterrace`)
  and under `links.roots`, and rechecks its id. A link is input from outside:
  one strict parser in `gitterrace-actions/src/link.rs`, nothing from a link is
  stored, and an unknown id never clones or fetches.
- **The MCP registry lists one bundle per target.** The registry takes a local
  stdio binary only as an `mcpb` package, a URL with "mcp" in it and the file's
  sha256. A bundle names operating systems, not architectures, so each dist
  target gets its own `.mcpb` with that one binary and `THIRD-PARTY-NOTICES`.
  A hand-written workflow in dist's global-artifacts slot packs them from the
  archives (`scripts/release/mcpb-pack.py`, byte for byte repeatable) and renders
  `scripts/release/mcpb/server.json`, so host ships them in the same `gh release create` as the
  archives and a release never exists without them. A publish job lists the
  version after host, with `mcp-publisher` at a pinned, verified version and the
  job's OIDC login, no secret. The registry refuses a version twice, so it is
  the last job and every check runs before it. `release-mcpb-check.py` holds the
  templates, the dist targets, the tag and the bytes in step. The registry name
  in the registry manifest is permanent.

## Cache

<!-- BEGIN GENERATED: cache -->
| File | Holds |
| --- | --- |
| `VERSION` | The cache format, `v` and the number. A mismatch deletes the cache and rebuilds it. The format is never migrated. |
| `lock` | Empty. Held locked by whichever process is extending the segment set, so two processes on one repository take turns instead of building on one manifest. |
| `owner` | Only in a cache outside the repository: the real path of the git dir it belongs to. Another path there wipes the cache, so a hash collision rebuilds. |
| `tip` | The commit id the segment files were last built for. Opening insights for that commit skips walking its history to check the files hold exactly it. |
| `lines.json` | Line counts per blob id for the language chart. A blob never changes, so the counts never go stale. |
| `chain.state` | Languages chain totals, checkpointed every 64 commits; a newer tip walks only new commits. Read only whole, under its rules key. |
| `checkpoints` | Snapshots of uncommitted work, append-only and checksummed. Capped by `review.checkpoint_mb`, oldest dropped. Not rebuildable. |
| `viewed` | Viewed marks per file and patch hash; an unmark is kept as a tombstone so a late migration cannot revive it. Atomic rename. Not rebuildable. |
| `findings` | The review basket: findings with notes, capped by `review.findings_entries`. Rewritten by atomic rename. Not rebuildable. |

The format version is `3`, stored as `v3`.

<!-- END GENERATED: cache -->

The cache lives in `.git/gitterrace/` when that folder is writable, else in
`$GITTERRACE_INDEX_FALLBACK/<hash of the git dir>` (the Docker image sets
`/cache/index`), else nowhere.

Cache folders are mode 0700 and files 0600, set at creation, so the copies of
uncommitted text in checkpoints and the notes in findings never depend on the
umask. A cache made by an older build is tightened when opened, through
no-follow handles, so a planted link is never chmod'ed. A shared repository
serves each user from their own cache: another user cannot write
`.git/gitterrace`, so they use the user-cache folder.

<!-- BEGIN GENERATED: user-files -->
`checkpoints`, `viewed` and `findings` hold user state, so a wipe or an older
build spares them.

<!-- END GENERATED: user-files -->

A `viewed` file that does not decode lists as no marks, but a write
refuses with an error and leaves the bytes untouched, as `findings` does
for a damaged span.
A `findings` tail that is cut short is a torn write and the next rewrite drops
it, but only when nothing in it is a whole frame. A last frame whose length
field was raised past the end of the file looks the same, so a tail whose bytes
match the header's checksum counts as a damaged span: kept, listed as
incomplete, appended after and never rewritten. Every rewrite goes through
one function that refuses on a damaged span, so no path drops unread bytes.

A checkpoint is a framed record with zstd copies of the
changed text files. A torn tail is ignored, so a crash mid-write loses only
that checkpoint. The writer and the reader share one set of header bounds: the
writer drops the oldest whole checkpoints, or refuses a header the reader would
refuse, so nothing written is unreadable. Neither limit is silent: when the
64 MiB header budget forces a drop, the write result and the checkpoint list
carry the count of dropped checkpoints (kept in the newest header, so it
survives later writes). A checkpoint that reaches the 256 MiB restore budget
keeps the path and hash of the files whose copy it skipped and reports how many
(`copies_omitted`) in the create result and the list. A commit range walked for
a comparison stops at `limits.worktree_files` paths during the traversal, not
after it, and the comparison reports `files_omitted`.

The review basket is rewritten by atomic rename, except when a frame is
damaged: reads skip that frame, adds append, and removes are refused, so no
byte of it is lost. A note is at most 2000 UTF-8 bytes, and a longer one is
refused, never cut.

The index is a set of immutable files named by one manifest, swapped by rename,
so a crash leaves the previous complete set. Pages read bounded, checksummed
blocks and never load the history: memory while serving is a fixed budget
(`resources.cache_mb`), whatever the length of the history. Commit search reads
a compressed message column in the same files instead of walking git. If the
cache cannot be read it is deleted and rebuilt, never partly served.

How the files are laid out, how search reads them, the memory budget and the
seam pages read through are in
[the index internals](../crates/gitterrace-index/src/seg/internals.md), next to
the [store design](../crates/gitterrace-index/src/seg/store.md).

### How the languages page reads the index

The languages page charts lines per language along the first-parent history.
The page walks the first parents through the stored records, newest first, and
reads each walked commit's file rows in bounded batches.

Rows are keyed by path, and the language and path filters apply when the page is
built, so changing them needs no rebuild.

Format numbers: the cache format (`VERSION`, `FORMAT_VERSION` in `store.rs`) is
stored with a `v` prefix, and a bare number without it is reset, not
refused. The container byte version (`seg/format.rs`) is separate.

### How hotspots and ownership read the index

A per-path aggregate cannot serve these pages, since a renamed or reused name
belongs to two files (over half of rust's files at HEAD). Every update writes
`tip-<gen>.gts` (`tip.rs`): each commit's file rows keyed by the name at the
indexed tip, with churn. Hotspots, ownership, coupling and code age read it, and
a set without it reads the commit rows directly.

`crates/gitterrace-index/tests/latency.rs` holds today's times on a 100k-commit
synthetic history as ceilings, and
`crates/gitterrace-web/tests/memory_budget.rs` guards serving memory on
synthetic repositories of growing size.

Every background task that fills a cache (the warmers, the related-root walk
after a HEAD move) holds the warming guard from before it is spawned until it
ends, and the idle release defers while any guard is held. A release that ran
between such a task's reads and its stores would leave the caches full until the
next period.

Concurrency caps (page jobs, live streams, the raw budget) are one `Slots`
type, defined in `gitterrace-api` so the API's job reads share it and no holder
can reach the raw semaphore. A cut that finds slots busy is owed, and each slot
that returns retires part of the debt, so a lowered `[limits]` or `[server]` value always lands. A
raise pays the debt off first.

The bench harness, corpus and results are in [benchmarks](benchmarks.md).

## Routes

Every row from `/` to `/api/v1` is per repository and lives under `/r/{name}`,
also when one repository is served. The rest are the server's own and never take
the prefix.

<!-- BEGIN GENERATED: routes -->
| Route | Shows |
| --- | --- |
| `/` | README, last commit, file list, key numbers. The server's own `/` forwards to the one repository, or lists several |
| `/tree`, `/blob`, `/raw`, `/blame` `/{rev}/{path}` | Files: listing, highlighted view (large files load in chunks), bytes, authorship |
| `/log/{rev}`, `/history/{path}`, `/compare` | Commits, one file's history, two revisions |
| `/commit/{id}`, `/diff/{rev}/{index}`, `/diff-find/{rev}` | A commit's diff, large files fold and load on demand |
| `/refs`, `/tag/{name}`, `/worktrees`, `/stashes`, `/worktree`, `/worktree/diff/{path}`, `/worktree/find`, `/worktree/raw/{path}`, `/worktree/blob/{path}` | Git state: branches, tags, worktrees, stashes and uncommitted changes, each file as diff, bytes or page |
| `/search` | Code and commit search |
| `/stats`, `/stats/{page}` | Insights on the shared filter and period: activity (with commit types and code frequency), contributors, hotspots (with patterns and coupling), languages, ownership (with knowledge risk, and one page per person). `?rev=` follows any ref |
| `/events`, `/api/{files,refs,last,insight,progress}` | Live updates and the fragments pages fetch, such as a file's card |
| `/api/v1/...` | JSON API, see [api.md](api.md) |
| `/go/{repo-id}/{sha}/{path}` | Permalink: finds the served repository by id |
| `/_send/targets` | Send to targets as JSON template parts, filled in by the page |
| `/r/{name}/...`, `/api/repos` | The prefix of every per-repository row above, also when one is served; and the server-wide list the switcher reads |
| `/_dirty` | The repositories with uncommitted work, as JSON the home page fetches to fill its chips after the rows render |
| `POST /_repos`, `DELETE /_repos`, `GET /_repos?nonce=N` | Adds a repository to the running server for `git terrace open`, or takes one off for `git terrace remove`: loopback, no browser, secret from a 0600 file. The GET proves the secret by hashing it with the nonce |
| `POST /_checkpoint`, `GET`/`POST /_viewed`, `POST /_viewed/all` | The working tree page's writes: take a checkpoint, mark files viewed, read the marks without the API. Same-origin with the owner cookie, local servers only, and they write only `.git/gitterrace` |
| `/_findings` | The review basket: `GET` lists it with where each finding's code is now, `POST` adds one, `DELETE` removes some or all. Writes are same-origin with the owner cookie, local servers only, and touch only `.git/gitterrace` |
| `/_launch` | Owner launch: `POST` hands the holder of the 0600 secret a one-time ticket, `GET ?t=` spends it and sets the HttpOnly owner cookie every write needs |
| `/_settings`, `/_settings/file`, `POST /_settings/check` | The settings page: a form and a plain editor for `config.toml` and `actions.toml`, read, checked and saved on local servers only, same-origin with the owner cookie, never a path from the request |

<!-- END GENERATED: routes -->

## Security guards

Each door in [security](security.md) and what holds it in the code:

| Door                       | Holds it                                                         | Test                                                   |
| -------------------------- | ---------------------------------------------------------------- | ------------------------------------------------------ |
| Every request              | `guard.rs`: Host allowlist, `Sec-Fetch-Site`, nosniff, CORP, CSP | `guard.rs` tests, `security_class_tests`               |
| Page routes (`PUBLIC`)     | `restrict_to_committed` when public, time bound                  | `public.rs` tests                                      |
| Local routes (`LOCAL`)     | `only_public` 404s anything not in `PUBLIC`                      | `every_local_route_is_dark_on_a_public_server`         |
| Path tails                 | Tree lookup, the worktree reader canonicalizes the parent        | `no_path_route_reads_outside_the_tree`                 |
| `/raw`                     | `sandbox` CSP for SVG and text, PDF attachment, size cap         | `wiring_tests`, `security_class_tests`                 |
| `POST /_actions/run`       | `same_origin_write`, JSON only, `actions/src/config/trust.rs`    | `run_route_security_table`                             |
| `/_settings/*`             | `same_origin_write`, enum file choice, stamp                     | `settings/tests.rs`                                    |
| `/_repos`                  | Add token from a 0600 file, browser headers refused              | `add/tests.rs`                                         |
| `/_launch`                 | One-time ticket for the 0600 secret, cookie HttpOnly Strict      | `owner.rs` tests, `launch_*` in `add/tests.rs`         |
| Every non-GET method       | Forged writes without the owner cookie never succeed             | `a_forging_loopback_client_without_owner_proof_writes_nothing` |
| Every page                 | Holds no token                                                   | `no_served_page_contains_a_write_token`                 |
| MCP server                 | Read-only tools, the switches of `/api/v1`                       | `mcp_tools_follow_the_switches_of_the_web_api`         |
| `open` and `gitterrace://` | Strict link parse, known clones only                             | `open` tests                                           |
| Static export              | `_headers` carries the CSP, no local routes crawled              | `export/headers_tests.rs`                              |
| Hostile content            | Askama escaping, CSP, actions never from the repository          | `hostile_repo_content_is_escaped_where_the_menu_lives` |

`security_class_tests.rs` reads every route from the built routers, so a route
registered anywhere is walked: public or `LOCAL`, in the sitemap, dark when
public, headers on every method, path escapes in the tail and in `?path=`,
`?file=` and `?doc=`, and tokenless writes.

## Syntax highlighting

Code, diffs, blame and fenced blocks in markdown are highlighted with the
grammars of [bat](https://github.com/sharkdp/bat) (through `two-face` and
`syntect`), compiled into the binary. A file is matched by name, then extension,
then its `#!` line. Text over `limits.highlight_kb` shows plain. These are the
languages:

<!-- BEGIN GENERATED: languages -->
182 languages: ActionScript, Ada, Apache Conf, AppleScript, ARM Assembly,
AsciiDoc (Asciidoctor), ASP, Authorized Keys, AWK, Batch File, BibTeX, Bourne
Again Shell (bash), C, C#, C++, Cabal, CFML, Clojure, CMake, CMake C Header,
CMake C++ Header, CMakeCache, CoffeeScript, Command Help, CpuInfo, Crontab,
Crystal, CSS, D, Dart, debsources, Diff, Dockerfile, DotENV, Elixir, Elm, Email,
Erlang, F#, Fish, Fortran (Fixed Form), Fortran (Modern), Fortran Namelist,
fstab, GDScript (Godot Engine), Git Attributes, Git Commit, Git Config, Git
Ignore, Git Link, Git Log, Git Mailmap, Git Rebase Todo, GLSL, gnuplot, Go,
Gomod, Gosum, GraphQL, Graphviz (DOT), Groff/troff, Groovy, group, Haskell,
Highlight non-printables, Hosts File, HTML, HTML (ASP), HTML (EEx), HTML
(Erlang), HTML (Jinja2), HTML (Rails), HTML (Tcl), HTML (Twig), HTTP Request and
Response, Idris, INI, Java, Java Properties, Java Server Page (JSP), JavaScript,
JavaScript (Babel), JavaScript (Rails), Jinja2, JQ, JSON, jsonnet, Julia, Known
Hosts, Kotlin, LaTeX, Lean 4, Less, Lisp, Literate Haskell, LiveScript, LLVM,
log, Lua, Makefile, Manpage, Markdown, MATLAB, MediaWiki, MemInfo, NAnt Build
File, nginx, Nim, Ninja, Nix, NSIS, Objective-C, Objective-C++, OCaml, OCamllex,
OCamlyacc, Odin, orgmode, Pascal, passwd, Perl, PHP, PowerShell, Protocol
Buffer, Protocol Buffer (TEXT), Puppet, PureScript, Python, QML, R, Racket, Rd
(R Documentation), Rego, Regular Expression, Requirements.txt, resolv,
reStructuredText, Robot Framework, Ruby, Ruby Haml, Ruby on Rails, Ruby Slim,
Rust, Salt State (SLS), Sass, Scala, SCSS, Separated Values, SML, Solidity, SQL,
SQL (Rails), SSH Config, SSHD Config, Strace, Stylus, Svelte, Swift, syslog,
SystemVerilog, Tab Separated Values, Tcl, Terraform, TeX, Textile, Todo.txt,
TOML, TypeScript, TypeScriptReact, Typst, varlink, Verilog, VHDL, VimHelp, VimL,
Vue Component, Vyper, WGSL, x86_64 Assembly, XML, YAML, Zig.

<!-- END GENERATED: languages -->
