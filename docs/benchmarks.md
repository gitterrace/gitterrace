# Benchmarks

gitterrace should feel instant on large repositories. Each run is kept, named by
date and CPU, and each warm time is compared with the previous run.

## What is measured

Release build only. Every page metric is the median of N warm runs (default 5)
with min and max. Cold is the first request after the index is ready, with the
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
3. **Idle.** CPU percent over a 60 s window with no client, and again with one
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
run plus headroom, except three product goals: a warm page in 300 ms, idle at 1%
CPU and the huge index in 600 s. The time to ready holds the index budget until
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
measured it, and names that run in the change column. The newest three runs are
kept.

Serving memory does not grow with the history: once the index is ready, idle
footprint stays at most 48 MB above a just-started server (`footprint_idle_mb`,
the 32 MB `resources.cache_mb` plus 16 MB), on every size and after every
insights page. A peak during the index build is fine. `make memory-check`
measures it on a real clone.

Runs are made on a quiet machine: the harness waits for it to settle, and
`make docs-check` refuses results from a busy one. The published run is made
once per release, on the release candidate commit.

### Insights pages on a large history

The pages that read every change of every file have a cold target for the first
request after the index is ready. Hotspots and ownership hold at most 1.5 times
the time of building them in memory. Languages has an absolute goal. Measured on
rust (345k commits, 64k files at HEAD), release build, M1 Max, median of 3 cold
runs:

| Page      | Period | Target | Measured | Result |
| --------- | ------ | ------ | -------- | ------ |
| Hotspots  | all    | 230 ms | 185 ms   | met    |
| Hotspots  | 1 year | 230 ms | 144 ms   | met    |
| Ownership | all    | 115 ms | 79 ms    | met    |
| Ownership | 1 year | 115 ms | 35 ms    | met    |
| Languages | -      | 66 ms  | 3 ms     | met    |

Languages was 589 ms when its chain walk (every first-parent commit of the
history) ran in the first visitor's request. The warm-up now builds the chain
and renders the page before the repository turns ready, so that request is a
memo hit. The walk's cost moved into the warm-up (about 5 s from the index
holding HEAD to ready on rust, with every page), and `the_first_page_after_ready_is_served_from_the_warm_up`
fails on a render or a chain walk after ready. The server renders these pages
right after the index is ready, so most visitors get them from memory in a few
milliseconds. `make speed-check` measures them on the rust and
linux clones and fails on a page over its target. The overview, statistics,
contributors and hotspot period pages are rendered before the repository turns
ready, so their target is a memory hit's, 300 ms.

### Commit search with the message column

`index.has_messages` keeps every commit message in the index. Measured on rust
(345,460 commits), release build, M1 Max, median of 5, page cache warm, with the
default `limits.search_commits` of 200:

| Query                             | Column on | Column off |
| --------------------------------- | --------- | ---------- |
| Literal hit (`Option`)            | 68 ms     | 108 ms     |
| Literal miss                      | 68 ms     | 3134 ms    |
| Regex hit (`fix(es\|ed)? #\d{5}`) | 80 ms     | 596 ms     |
| Regex miss                        | 58 ms     | 3361 ms    |

The cost: the cache grows from 256 MB to 287 MB (12%). Turning it on takes one
16 s pass. An update of 100 commits takes 11 s.

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
Run 2026-11-22 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace v0.2.0-262128e, git version 2.54.0 (Apple Git-157).
Measured on a quiet machine.

- **rust:** not measured on a quiet machine (other processes used 5 or more
  cores during export), so its numbers are withheld.

| Repo | Metric | Value | Budget | Status | Change against an earlier run |
| --- | --- | --- | --- | --- | --- |
| control | index build | 0.4 s (0.4 to 0.5) | 30 s | pass | 2% slower (vs 2026-11-21) |
| control | warm page | 4 ms | 300 ms | pass | 90% slower (vs 2026-11-21) |
| control | cold page | 29 ms (29 to 31) | 1500 ms | pass | 13% slower (vs 2026-11-21) |
| control | peak memory | 100 MB | 500 MB | pass | 4% more memory (vs 2026-11-21) |
| control | export | 29.0 s (28.8 to 29.3) | 60 s | pass | 3% faster (vs 2026-11-21) |
| git | index build | 10.8 s (10.4 to 11.4) | 300 s | pass | unchanged (vs 2026-11-21) |
| git | warm page | 12 ms | 300 ms | pass | 2% faster (vs 2026-11-21) |
| git | cold page | 2377 ms (2372 to 2383) | 3000 ms | pass | unchanged (vs 2026-11-21) |
| git | peak memory | 407 MB | 1500 MB | pass | 6% more memory (vs 2026-11-21) |
| git | export | 25.5 s (25.4 to 25.6) | 300 s | pass | 2% faster (vs 2026-11-21) |
| rust | index build | withheld (load) | - | withheld | - |
| rust | warm page | withheld (load) | - | withheld | - |
| rust | cold page | withheld (load) | - | withheld | - |
| rust | peak memory | withheld (load) | - | withheld | - |
| rust | export | withheld (load) | - | withheld | - |

<!-- END GENERATED: results -->
