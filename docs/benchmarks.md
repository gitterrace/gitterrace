# Benchmarks

gitterrace should feel instant on large repositories. Each run is kept, named by
date and CPU, and each warm time is compared with the previous run.

## What is measured

Release build only. Every page metric is the median of the warm runs with
min and max. Cold is the first request after the index is ready, with the
OS file cache warmed by one untimed read of the repository. Warm is the runs
after it.

1. **Index build.** Seconds from process start until `/api/progress` leaves
   indexing (the index holds HEAD), from an empty cache, plus peak RSS sampled
   meanwhile and the size of the cache on disk. **Add to ready** is the seconds
   until it reports ready: the index build plus the warm-up, which renders the
   first pages before a visitor asks for them.
2. **Pages.** Time to first byte and full response for `/`, `/tree/HEAD`, a deep
   tree path, `/blob` of the largest file under the text limit, `/blame` of that
   file, `/log/HEAD`, `/commit` and `/diff` fragment of the newest commit,
   `/search` for a common identifier, `/refs`, and every `/stats` page. Every
   page must answer 200, with an empty config directory.
3. **Idle.** CPU percent over a window with no client, and again with one
   `/events` client open, which is the cost of the live watcher.
4. **Memory.** RSS after the index is ready and after every page, then the dirty
   footprint (`footprint` on macOS, `Private_Dirty` on Linux) after the idle
   release, and that of a just-started server.
5. **Export.** `git terrace export` time and output size, on every repository,
   written to `export-<repo>` under the clone directory and removed after.

## Why these repositories

Each is pinned to a sha in `tests/bench/corpus.toml`, so a re-run measures the
same history.

<!-- BEGIN GENERATED: corpus -->
- [**control**](https://github.com/gitterrace/gitterrace): this repository, the floor.
- [**git**](https://github.com/git/git): medium, long-lived, a classic reference.
- [**rust**](https://github.com/rust-lang/rust): huge history, many files and contributors.
- [**linux**](https://github.com/torvalds/linux): the extreme, opt-in (`make bench LARGE=1`), about 30 GB.

<!-- END GENERATED: corpus -->

Clones are blobless fetches of the pinned sha, kept in `$GITTERRACE_BENCH_DIR`
(default `~/.cache/gitterrace-bench`). The harness refuses to clone when free
disk is under the corpus `need_gb`.

## Budgets

Each size class has a target per metric in the corpus file: the first measured
run plus headroom, except the product goals: the warm page, idle CPU and the
huge index. The time to ready holds the index budget until
a run calibrates it. An index time, time to ready or memory over budget fails
the run. Any other metric over budget fails it only with `BENCH_STRICT=1`.

The memory budget (`rss_mb`, shown as peak memory) gates physical footprint,
not RSS: macOS `phys_footprint`, on Linux the larger of `RssAnon` and private
dirty pages. RSS is recorded beside it. Peak memory is the largest of the
server's footprint during the index build, the index child's footprint
(`peak_child_footprint_mb` in the index result) and the server's footprint after
the pages. The index build runs in a child process, so a measure without the
child leaves the build out. The server's own footprint includes the warm-up,
which runs before the repository turns ready. A run from before the child was
recorded measured something else, so the comparison table calls its peak memory
a new measure and never puts a percentage on it.

The comparison table sets each repo against the newest earlier kept run that
measured it, and names that run in the change column. A memory sample under
25% of the median of the repo's other recorded runs is a missed sample. The
report marks it invalid, names it, and compares against the newest valid run
instead. The result file keeps it.

<!-- BEGIN GENERATED: budgets -->
| Class | Warm ms | Cold ms | Index s | Ready s | Idle CPU % | Peak MB | Idle MB | Export s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| small | 300 | 1500 | 30 | 30 | 1 | 500 | 48 | 60 |
| medium | 300 | 3000 | 300 | 300 | 1 | 1500 | 48 | 300 |
| large | 300 | 10000 | 2400 | 2400 | 1 | 6000 | 48 | - |
| huge | 500 | 20000 | 600 | 600 | 1 | 8000 | 48 | - |

A published run takes the median of 5 warm runs and idles over a 60 s window.
The newest 3 runs are kept.

<!-- END GENERATED: budgets -->

Serving memory does not grow with the history: once the index is ready, idle
footprint stays within `footprint_idle_mb` of a just-started server (the cache
budget `resources.cache_mb` plus a margin), on every size and after every
insights page. A peak during the index build is fine. `make memory-check`
measures it on a real clone.

Runs are made on a quiet machine: the harness waits for it to settle, and
`make docs-check` refuses results from a busy one. The published run is made
once per release, on the release candidate commit.

### Insights pages on a large history

The pages that read every change of every file have a cold target for the first
request after the index is ready. Hotspots and ownership hold at most 1.5 times
the time of building them in memory. Languages has an absolute goal. The
targets:

<!-- BEGIN GENERATED: gates -->
| Repository | Page | Target (ms) |
| --- | --- | --- |
| rust | languages | 66 |
| rust | hotspots-all | 230 |
| rust | hotspots-1y | 300 |
| rust | hotspots-90d | 300 |
| rust | ownership-all | 115 |
| rust | ownership-1y | 115 |
| rust | overview | 300 |
| rust | stats-overview | 300 |
| rust | contributors | 300 |
| linux | hotspots-1y | 300 |
| linux | hotspots-90d | 300 |
| linux | overview | 300 |
| linux | stats-overview | 300 |
| linux | contributors | 300 |

<!-- END GENERATED: gates -->

The warm-up builds the languages chain and renders the page before the
repository turns ready, so the first request is a memo hit. The walk's cost
sits in the warm-up, and `the_first_page_after_ready_is_served_from_the_warm_up`
fails on a render or a chain walk after ready. The server renders these pages
right after the index is ready, so most visitors get them from memory in a few
milliseconds. `make speed-check` measures them on the rust and linux clones
and fails on a page over its target. The overview, statistics,
contributors and hotspot period pages are rendered before the repository turns
ready, so their target is a memory hit's, 300 ms.

### Commit search with the message column

`index.has_messages` keeps every commit message in the index, so a commit
search reads a column instead of walking git. The cache grows and turning it on
takes one pass: both are in the results above.

## Reproduce

```sh
make bench          # control, git, rust: network and about 20 GB of disk
make bench LARGE=1  # adds linux
make bench-report   # rewrite the results block from the newest run
make gc BENCH=1     # remove the clones (DRY=1 only prints path and size)
```

`make check` runs only `bench-guard`, a smoke run on a tiny repository.

Each run names the commit it measured, from `--version`. A `-dirty` build is
never published: `bench-report --check` refuses it. The JSON keeps the commit in
`machine.commit` and a `dirty` flag.

## Results

The newest run against its budgets. The numbers of each kept run are in
[tests/bench/results](../tests/bench/results/), and `make bench-report` draws
their charts and pages there.

<!-- BEGIN GENERATED: results -->
Run 2026-11-25 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace v0.2.1-e2587e5, git version 2.54.0 (Apple Git-157).
Measured on a quiet machine.

| Repo | Metric | Value | Budget | Status | Change against an earlier run |
| --- | --- | --- | --- | --- | --- |
| control | index build | 0.5 s (0.4 to 0.7) | 30 s | pass | unchanged (vs 2026-11-23) |
| control | warm page | 2 ms | 300 ms | pass | 8% slower (vs 2026-11-23) |
| control | cold page | 53 ms (3 to 54) | 1500 ms | pass | 76% slower (vs 2026-11-23) |
| control | peak memory | 49 MB | 500 MB | pass | 55% less memory (vs 2026-11-23) |
| control | export | 31.4 s (30.6 to 31.4) | 60 s | pass | 5% faster (vs 2026-11-23) |
| git | index build | 12.7 s (12.3 to 13.0) | 300 s | pass | 20% slower (vs 2026-11-23) |
| git | warm page | 13 ms | 300 ms | pass | 2% faster (vs 2026-11-23) |
| git | cold page | 2416 ms (2414 to 2434) | 3000 ms | pass | 1% slower (vs 2026-11-23) |
| git | peak memory | 390 MB | 1500 MB | pass | 2% less memory (vs 2026-11-23) |
| git | export | 25.8 s (25.2 to 25.8) | 300 s | pass | 2% faster (vs 2026-11-23) |
| rust | index build | 111.2 s (101.1 to 124.8) | 2400 s | pass | 35% slower (vs 2026-11-23) |
| rust | warm page | 31 ms | 300 ms | pass | 4% slower (vs 2026-11-23) |
| rust | cold page | 1466 ms (1455 to 1495) | 10000 ms | pass | 2% faster (vs 2026-11-23) |
| rust | peak memory | 917 MB | 6000 MB | pass | 3% less memory (vs 2026-11-23) |

<!-- END GENERATED: results -->
