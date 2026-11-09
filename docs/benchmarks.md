# Benchmarks

gitterrace should feel instant on large repositories. This page says what is
measured, how, and what the latest run found. The harness is
`scripts/bench-run.py`, the corpus is `tests/bench/corpus.toml`, and results land in
`tests/bench/results/<date>-<machine>.json`, where the machine label comes from the
CPU (for example `apple-m1-max`), never the hostname. Older files stay as
history: the page compares each warm time with the previous run.

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
history. The goal is at most the 32 MB cache budget plus 16 MB of margin above a
just-started server, on every repository size, also after a sweep of every
insights page. Page cache for the index files is not counted. A peak during the
index build is fine. `crates/gitterrace-web/tests/memory_budget.rs` guards it on
synthetic repositories of growing size, and `make memory-check` measures it on a
real clone. Every bench run checks it too, as the `footprint_idle_mb` budget:
idle footprint minus that of a just-started server, at most 48 MB on every
class.

Timings need a quiet machine. Before each timed phase the harness waits until
the 1-minute load average stays under `BENCH_MAX_LOAD` (default half the cores)
for 20 s, and records the load per phase. `make docs-check` refuses results
timed above that limit.

The published run is made once per release, on the release candidate commit, on
a quiet machine, never as extra runs in between. The 0.1.0 run is the first
baseline.

### File pages on a large history: target missed

The pages that read every change of every file (hotspots, ownership) have a cold
target of at most 1.5 times the in-memory build they replaced: on rust, hotspots
230 ms and ownership 115 ms for the first request after the index is ready. The
streaming store misses it. Measured on rust (345k commits, 64k files at HEAD),
release build, M1 Max, first request after ready, 1-minute load 11 to 16:

| Page      | Period | Target | Measured        |
| --------- | ------ | ------ | --------------- |
| Hotspots  | all    | 230 ms | 950 to 1170 ms  |
| Hotspots  | 1 year | 230 ms | 640 to 790 ms   |
| Ownership | all    | 115 ms | 1050 to 1330 ms |
| Ownership | 1 year | 115 ms | 520 to 640 ms   |

The server renders these pages right after the index is ready, so a visitor who
arrives after that warm-up (about 10 s on rust) gets them from the page memo in
a few milliseconds. The cold numbers are what a visitor pays inside that window,
or for a filter combination that was not rendered before.

The cost is reading every file row once per page with renames followed. A
per-path aggregate cannot replace that read today: it is keyed by the name a
file had in each commit, and a name that was renamed or reused belongs to two
files at different times. On rust more than half the files at HEAD have such a
name. Ownership over all history reads the aggregate for the files that were
never renamed and the rows for the rest. An aggregate keyed by the name at the
indexed tip, rebuilt when a new rename moves old rows, is the planned fix.
`crates/gitterrace-index/tests/latency.rs` holds today's times on a 100k-commit
synthetic history as ceilings, so the read path cannot get slower unnoticed.

### Commit search with the message column

`index.has_messages` keeps every commit message in the index, so commit
search reads the cache instead of walking git. Measured on rust (345,460
commits) at the corpus sha, release build, M1 Max, 1-minute load 28 at the
start, median of 5, page cache warm, through the search handler with the
default cap of 200 rows:

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

## Not measured

Browser rendering and JavaScript, network latency (the server is local),
concurrent clients, the worktree view, incremental index updates after new
commits, and a spinning disk.

## Reproduce

```sh
make bench          # control, git, rust. Needs the network and about 20 GB of disk
make bench LARGE=1  # adds linux
make bench-report   # rewrite the block below from the newest tests/bench/results file
make gc BENCH=1     # print the path and size, then remove the clones (DRY=1 only prints)
```

`make check` runs only `bench-guard`, which drives the harness once against a
tiny generated repository to prove it still works.

## Results

The newest run against its budgets. The charts and every page of each kept run
are in tests/bench/results.

<!-- BEGIN GENERATED: results -->
Run 2026-11-03 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace v0.1.0, git version 2.54.0 (Apple Git-157). Every
timed phase started with the 1-minute load average under 5. The -dirty label
comes from the version stamp, which counted a bench file as an uncommitted
change. The binary was built from the clean commit. The rust and linux exports
failed at the page cap and are shown as failed.

- **rust:** not measured on a quiet machine (other processes used 5 or more
  cores during export, export#3, index), so its numbers are withheld.

| Repo | Metric | Value | Budget | Status |
| --- | --- | --- | --- | --- |
| control | index build | 0.2 s (0.2 to 0.3) | 30 s | pass |
| control | warm page | 11 ms | 300 ms | pass |
| control | cold page | 23 ms (22 to 24) | 1500 ms | pass |
| control | peak physical footprint (index peak or after pages) | 40 MB | 500 MB | pass |
| control | export | 12.3 s (12.0 to 12.3) | 60 s | pass |
| git | index build | 10.8 s (10.5 to 11.3) | 300 s | pass |
| git | warm page | 27 ms | 300 ms | pass |
| git | cold page | 2355 ms (2321 to 2381) | 3000 ms | pass |
| git | peak physical footprint (index peak or after pages) | 383 MB | 1500 MB | pass |
| git | export | 44.4 s (42.8 to 45.9) | 300 s | pass |
| linux | index build | 1596.4 s (1596.4 to 1596.4) | 600 s | over |
| linux | warm page | 43 ms | 500 ms | pass |
| linux | cold page | 27674 ms (27674 to 27674) | 20000 ms | over |
| linux | peak physical footprint (index peak or after pages) | 6449 MB | 8000 MB | pass |
| linux | export | failed | - | failed |
| rust | index build | withheld (load) | - | withheld |
| rust | warm page | withheld (load) | - | withheld |
| rust | cold page | withheld (load) | - | withheld |
| rust | peak physical footprint (index peak or after pages) | withheld (load) | - | withheld |
| rust | export | withheld (load) | - | withheld |

Every number, chart and page of each run, as files:
[tests/bench/results](../tests/bench/results/README.md).

<!-- END GENERATED: results -->
