# Benchmarks

gitterrace should feel instant on large repositories. Each run is kept, named
by date and CPU, and each warm time is compared with the previous run.

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
   `/search` for a common identifier, `/refs`, and every `/stats` page. Every
   page must answer 200, with an empty config directory.
3. **Idle.** CPU percent over a 60 s window with no client, and again with one
   `/events` client open, which is the cost of the live watcher.
4. **Memory.** RSS after the index is ready and after every page, then the
   dirty footprint (`footprint` on macOS, `Private_Dirty` on Linux) after the
   idle release, and that of a just-started server.
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
run plus headroom, except three product goals: a warm page in 300 ms, idle at
1% CPU and the huge index in 600 s. An index time or memory over budget fails
the run. Any other metric over budget fails it only with `BENCH_STRICT=1`.

The memory budget (`rss_mb`) gates physical footprint, not RSS: macOS
`phys_footprint`, on Linux the larger of `RssAnon` and private dirty pages.
RSS is recorded beside it.

Serving memory does not grow with the history: once the index is ready, idle
footprint stays at most 48 MB above a just-started server (`footprint_idle_mb`,
the 32 MB `resources.cache_mb` plus 16 MB), on every size and after every
insights page. A peak during the index build is fine. `make memory-check`
measures it on a real clone.

Runs are made on a quiet machine: the harness waits for it to settle, and
`make docs-check` refuses results from a busy one. The published run is made
once per release, on the release candidate commit. The 0.1.0 run is the first
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

Languages is over its goal, and the cause is not yet known. The server renders
these pages right after the index is ready, so most visitors get them from
memory in a few milliseconds. `make speed-check` measures them on the rust
clone and fails on a page over its target.

### Commit search with the message column

`index.has_messages` keeps every commit message in the index. Measured on rust
(345,460 commits), release build, M1 Max, median of 5, page cache warm, with
the default `limits.search_commits` of 200:

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
never published: `bench-report --check` refuses it. The JSON keeps the commit
in `machine.commit` and a `dirty` flag.

## Results

The newest run against its budgets. The charts and every page of each kept run
are in tests/bench/results.

<!-- BEGIN GENERATED: results -->
Run 2026-11-12 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace v0.1.0-548f044, git version 2.54.0 (Apple Git-157).
Measured on a quiet machine.

| Repo | Metric | Value | Budget | Status | Change against previous |
| --- | --- | --- | --- | --- | --- |
| control | index build | 0.8 s (0.6 to 0.8) | 30 s | pass | 52% slower |
| control | warm page | 2 ms | 300 ms | pass | 30% faster |
| control | cold page | 25 ms (25 to 25) | 1500 ms | pass | 62% slower |
| control | peak memory | 117 MB | 500 MB | pass | 350% more memory |
| control | export | 34.4 s (33.5 to 34.7) | 60 s | pass | 48% slower |
| git | index build | 22.6 s (22.1 to 23.2) | 300 s | pass | 113% slower |
| git | warm page | 87 ms | 300 ms | pass | 145% slower |
| git | cold page | 2427 ms (2409 to 2448) | 3000 ms | pass | 4% slower |
| git | peak memory | 366 MB | 1500 MB | pass | 10% less memory |
| git | export | 55.1 s (53.1 to 56.8) | 300 s | pass | 8% slower |
| rust | index build | 97.1 s (96.9 to 98.0) | 2400 s | pass | 44% slower |
| rust | warm page | 122 ms | 300 ms | pass | 156% slower |
| rust | cold page | 1736 ms (1720 to 1737) | 10000 ms | pass | 6% slower |
| rust | peak memory | 914 MB | 6000 MB | pass | 1% more memory |
| linux | index build | 477.0 s (477.0 to 477.0) | 600 s | pass | 59% slower |
| linux | warm page | 45 ms | 500 ms | pass | 1% slower |
| linux | cold page | 3887 ms (3887 to 3887) | 20000 ms | pass | 55% faster |
| linux | peak memory | 1937 MB | 8000 MB | pass | 3% less memory |

Every number, chart and page of each run, as files:
[tests/bench/results](../tests/bench/results/README.md).

<!-- END GENERATED: results -->
