# Benchmarks

gitterrace should feel instant on large repositories. This page says what is
measured, how, and what the latest run found. Each run is kept, named by date
and CPU, and each warm time is compared with the previous run.

## What is measured

Release build only. Every page metric is the median of N warm runs (default 5)
with min and max. Cold is the first request after the index is ready, with the
OS file cache warmed by one untimed read of the repository. Warm is the runs
after it.

1. **Index build.** Seconds from process start until `/api/progress` reports
   ready, from an empty cache, plus peak RSS sampled meanwhile and the size of
   the cache on disk.
2. **Pages.** Time to first byte and full response for `/`, `/tree/HEAD`, a deep
   tree path, `/blob` of the largest file under the text limit, `/blame` of that
   file, `/log/HEAD`, `/commit` and `/diff` fragment of the newest commit,
   `/search` for a common identifier, `/refs`, and every `/stats` page. The
   probes are discovered from the pinned tree. Every page must answer 200 or the
   run fails, and the server runs with an empty config directory so the
   developer's own settings never apply.
3. **Idle.** CPU percent over a 60 s window with no client, and again with one
   `/events` client open, which is the cost of the live watcher.
4. **Memory.** RSS after the index is ready and after every page was visited,
   then the dirty footprint (`footprint` on macOS, `Private_Dirty` from
   `smaps_rollup` on Linux) once `/api/progress` reports an idle release, and
   that of a just-started server. RSS includes the mapped pack and index files
   and the working set not yet released, the footprint is the dirty memory the
   server still holds after the idle release.
5. **Export.** `git terrace export` time and output size, on every repository.
   The output goes to `export-<repo>` under the clone directory and is removed
   afterwards.

## Why these repositories

Each is pinned to a sha in the corpus file, so a re-run measures the same
history. The list comes from `tests/bench/corpus.toml`, each name links to its public
source.

<!-- BEGIN GENERATED: corpus -->
- [**control**](https://github.com/gitterrace/gitterrace): this repository, the floor.
- [**git**](https://github.com/git/git): medium, long-lived, a classic reference.
- [**rust**](https://github.com/rust-lang/rust): huge history, many files and contributors.
- [**linux**](https://github.com/torvalds/linux): the extreme, opt-in (`make bench LARGE=1`), about 30 GB.

<!-- END GENERATED: corpus -->

Clones are bare, blobless fetches of the pinned sha followed by `git backfill`,
kept in `$GITTERRACE_BENCH_DIR` (default `~/.cache/gitterrace-bench`), never in
the repository. The harness refuses to clone when free disk is under the corpus
`need_gb`.

## Budgets

Each size class has a target per metric (warm page, cold page, index time, idle
CPU, RSS, export) in the corpus file. They come from the first measured run plus
headroom, except the warm page target of 300 ms, the idle target of 1% CPU and
the huge index target of 600 s (a cold linux index in about 10 minutes), which
are product goals. The large and huge export budgets come from the first run
that exported those repos. A run marks each metric pass or over. Over budget
does not fail the run unless `BENCH_STRICT=1`.

The memory budget (`rss_mb`) gates physical footprint, not RSS: macOS
`phys_footprint`, and on Linux the larger of `RssAnon` and private dirty pages.
Clean mapped pack files count in RSS but not here, since the system takes them
back under pressure, and RSS is still recorded beside it.

Serving memory is a product goal: once the index is ready, a server that is
browsed or idle holds some MB, not GB, and that amount does not grow with the
history. The goal is at most the `resources.cache_mb` cache budget (default 32
MB) plus 16 MB of margin above a just-started server, on every repository size,
also after a sweep of every insights page. Page cache for the index files is
not counted. A peak during the index build is fine. A test guards it on
synthetic repositories of growing
size, and `make memory-check` measures it on a real clone. Every bench run
checks it too, as the `footprint_idle_mb` budget: idle footprint minus that of a
just-started server, at most 48 MB on every class.

Every run is made on a quiet machine, so runs compare under the same
conditions. The harness waits for the machine to settle before each timed
phase, and `make docs-check` refuses results from a busy one.

The published run is made once per release, on the release candidate commit, on
a quiet machine, never as extra runs in between. The 0.1.0 run is the first
baseline.

### Insights pages on a large history

The pages that read every change of every file have a cold target for the first
request after the index is ready. Hotspots and ownership hold at most 1.5 times
the in-memory build they replaced. Languages has an absolute goal. Measured on
rust (345k commits, 64k files at HEAD), release build, M1 Max, median of 3 cold
runs:

| Page      | Period | Target | Measured | Result |
| --------- | ------ | ------ | -------- | ------ |
| Hotspots  | all    | 230 ms | 185 ms   | met    |
| Hotspots  | 1 year | 230 ms | 144 ms   | met    |
| Ownership | all    | 115 ms | 79 ms    | met    |
| Ownership | 1 year | 115 ms | 35 ms    | met    |
| Languages | -      | 66 ms  | 589 ms   | missed |

Hotspots and ownership read a per-file summary that the index writes for the
tip, so a page visits each change once instead of every row of the history.
Languages reads the line counts along the first-parent chain, also written at
index time, in one pass. That pass is still over its goal, and the cause is
not yet known.

The server renders these pages right after the index is ready, so a visitor who
arrives after that warm-up gets them from memory in a few milliseconds. The
cold numbers are what a visitor pays inside that window, or for a filter
combination that was not rendered before. `make speed-check` measures them on
the rust clone and fails on a page over its target. A test also holds the file
pages under fixed ceilings on a synthetic history, so they cannot get slower
unnoticed.

### Commit search with the message column

`index.has_messages` keeps every commit message in the index, so commit
search reads the cache instead of walking git. Measured on rust (345,460
commits) at the corpus sha, release build, M1 Max, median of 5, page cache
warm, through the search handler with the
`limits.search_commits` cap (default 200 rows):

| Query                             | Column on | Column off |
| --------------------------------- | --------- | ---------- |
| Literal hit (`Option`)            | 68 ms     | 108 ms     |
| Literal miss                      | 68 ms     | 3134 ms    |
| Regex hit (`fix(es\|ed)? #\d{5}`) | 80 ms     | 596 ms     |
| Regex miss                        | 58 ms     | 3361 ms    |

The cost: the column is 31 MB, the cache grows from 256 MB to 287 MB (12%). The
first build, the set already indexed and the setting just turned on, takes 16 s.
An update of 100 commits, set and column together, takes 11 s (the diffs of the
new commits dominate). A miss reads no commit object, a hit reads only the
commits it lists. The process peaked at 2.35 GB resident during the full set
build that preceded these numbers, and held 1.6 GB after the searches,
mostly the set build's allocator state.

## Reproduce

```sh
make bench          # control, git, rust. Needs the network and about 20 GB of disk
make bench LARGE=1  # adds linux
make bench-report   # rewrite the block below from the newest tests/bench/results file
make gc BENCH=1     # print the path and size, then remove the clones (DRY=1 only prints)
```

`make check` runs only `bench-guard`, which drives the harness once against a
tiny generated repository to prove it still works.

## Which build

Each run names the gitterrace commit it measured as `gitterrace
v0.1.0-<7-hex>`, from the binary's `--version`. The commit is kept in
`machine.commit` of the results JSON and in the version string, and the
results block below shows it. A run whose commit is not recorded renders as
the plain version. A `-dirty` build is never published: `bench-report --check`
refuses it, and the JSON carries the `dirty` flag, not the mark in the version.

## Results

The newest run against its budgets. The charts and every page of each kept run
are in tests/bench/results.

<!-- BEGIN GENERATED: results -->
Run 2026-11-09 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace 0.1.0, git version 2.54.0 (Apple Git-157).
Measured on a quiet machine.

| Repo | Metric | Value | Budget | Status | Change against previous |
| --- | --- | --- | --- | --- | --- |
| control | index build | 0.5 s (0.3 to 0.6) | 30 s | pass | 116% slower |
| control | warm page | 3 ms | 300 ms | pass | 75% faster |
| control | cold page | 16 ms (15 to 16) | 1500 ms | pass | 33% faster |
| control | peak memory | 26 MB | 500 MB | pass | 34% less memory |
| control | export | 23.3 s (22.7 to 23.8) | 60 s | pass | 90% slower |
| git | index build | 10.6 s (10.4 to 10.8) | 300 s | pass | 1% faster |
| git | warm page | 36 ms | 300 ms | pass | 30% slower |
| git | cold page | 2336 ms (2327 to 2336) | 3000 ms | pass | 1% faster |
| git | peak memory | 407 MB | 1500 MB | pass | 6% more memory |
| git | export | 51.2 s (50.6 to 52.3) | 300 s | pass | 15% slower |
| rust | index build | 67.4 s (67.2 to 68.4) | 2400 s | pass | new in this run |
| rust | warm page | 48 ms | 300 ms | pass | new in this run |
| rust | cold page | 1641 ms (1638 to 1641) | 10000 ms | pass | new in this run |
| rust | peak memory | 909 MB | 6000 MB | pass | new in this run |
| rust | export | stopped at the page cap | - | capped | - |
| linux | index build | 300.7 s (300.7 to 300.7) | 600 s | pass | 81% faster |
| linux | warm page | 44 ms | 500 ms | pass | 2% slower |
| linux | cold page | 8620 ms (8620 to 8620) | 20000 ms | pass | 69% faster |
| linux | peak memory | 2004 MB | 8000 MB | pass | 69% less memory |
| linux | export | stopped at the page cap | - | capped | - |

Every number, chart and page of each run, as files:
[tests/bench/results](../tests/bench/results/README.md).

<!-- END GENERATED: results -->
