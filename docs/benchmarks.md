# Benchmarks

gitterrace should feel instant on large repositories. This page says what is measured,
how, and what the latest run found. The harness is `scripts/bench-run.py`, the corpus is
`bench/corpus.toml`, and results land in `bench/results/<date>-<machine>.json`, where the
machine label comes from the CPU (for example `apple-m1-max`), never the hostname. Older files
stay as history: the page compares each warm time with the previous run.

## What is measured

Release build only. Every page metric is the median of N warm runs (default 5) with min and
max. Cold is the first request after the index is ready, with the OS file cache warmed by one
untimed read of the repository. Warm is the runs after it.

1. **Index build.** Seconds from process start until `/api/progress` reports ready, from an
   empty cache, plus peak RSS sampled meanwhile and the size of the cache on disk.
2. **Pages.** Time to first byte and full response for `/`, `/tree/HEAD`, a deep tree path,
   `/blob` of the largest file under the text limit, `/blame` of that file, `/log/HEAD`,
   `/commit` and `/diff` fragment of the newest commit, `/search` for a common identifier,
   `/refs`, and every `/stats` page. The probes are discovered from the pinned tree. Every page
   must answer 200 or the run fails, and the server runs with an empty config directory so the
   developer's own settings never apply.
3. **Idle.** CPU percent over a 60 s window with no client, and again with one `/events`
   client open, which is the cost of the live watcher.
4. **Memory.** RSS after the index is ready and after every page was visited, then the dirty
   footprint (`footprint` on macOS, `Private_Dirty` from `smaps_rollup` on Linux) once
   `/api/progress` reports an idle release, and that of a just-started server. RSS includes the
   mapped pack and index files and the working set not yet released, the footprint is the dirty
   memory the server still holds after the idle release.
5. **Export.** `git terrace export` time and output size, on every repository. The output goes
   to `export-<repo>` under the clone directory and is removed afterwards.

## Why these repositories

Each is pinned to a sha in the corpus file, so a re-run measures the same history. The list
comes from `bench/corpus.toml`, each name links to its public source.

<!-- BEGIN GENERATED: corpus -->
- [**control**](https://github.com/gitterrace/gitterrace): this repository, the floor.
- [**git**](https://github.com/git/git): medium, long-lived, a classic reference.
- [**rust**](https://github.com/rust-lang/rust): huge history, many files and contributors.
- [**linux**](https://github.com/torvalds/linux): the extreme, opt-in (`make bench LARGE=1`), about 30 GB.

<!-- END GENERATED: corpus -->

Clones are bare, blobless fetches of the pinned sha followed by `git backfill`, kept in
`$GITTERRACE_BENCH_DIR` (default `~/.cache/gitterrace-bench`), never in the repository. The
harness refuses to clone when free disk is under the corpus `need_gb`.

## Budgets

Each size class has a target per metric (warm page, cold page, index time, idle CPU, RSS,
export) in the corpus file. They come from the first measured run plus headroom, except the
warm page target of 300 ms, the idle target of 1% CPU and the huge index target of 600 s (a
cold linux index in about 10 minutes), which are product goals. The large
and huge export budgets come from the first run that exported those repos. A run marks each
metric pass or over. Over budget does not fail the run unless `BENCH_STRICT=1`.

Serving memory is a product goal: once the index is ready, a server that is browsed or idle
holds some MB, not GB, and that amount does not grow with the history. The goal is at most the
32 MB cache budget plus 16 MB of margin above a just-started server, on every repository size,
also after a sweep of every insights page. Page cache for the index files is not counted. A
peak during the index build is fine. `crates/gitterrace-web/tests/memory_budget.rs` guards it on
synthetic repositories of growing size, and `make memory-check` measures it on a real clone.
Every bench run checks it too, as the `footprint_idle_mb` budget: idle footprint minus that
of a just-started server, at most 48 MB on every class.

Timings need a quiet machine. Before each timed phase the harness waits until the 1-minute load
average stays under `BENCH_MAX_LOAD` (default half the cores) for 20 s, and records the load per
phase. `make docs-check` refuses results timed above that limit.

The published run is made once per release, on the release candidate commit, on a quiet
machine, never as extra runs in between. The 0.1.0 run is the first baseline.

### File pages on a large history: target missed

The pages that read every change of every file (hotspots, ownership) have a cold target of
at most 1.5 times the in-memory build they replaced: on rust, hotspots 230 ms and ownership
115 ms for the first request after the index is ready. The streaming store misses it. Measured
on rust (345k commits, 64k files at HEAD), release build, M1 Max, first request after ready,
1-minute load 11 to 16:

| Page | Period | Target | Measured |
| --- | --- | --- | --- |
| Hotspots | all | 230 ms | 950 to 1170 ms |
| Hotspots | 1 year | 230 ms | 640 to 790 ms |
| Ownership | all | 115 ms | 1050 to 1330 ms |
| Ownership | 1 year | 115 ms | 520 to 640 ms |

The server renders these pages right after the index is ready, so a visitor who arrives after
that warm-up (about 10 s on rust) gets them from the page memo in a few milliseconds. The cold
numbers are what a visitor pays inside that window, or for a filter combination that was not
rendered before.

The cost is reading every file row once per page with renames followed. A per-path aggregate
cannot replace that read today: it is keyed by the name a file had in each commit, and a name
that was renamed or reused belongs to two files at different times. On rust more than half the
files at HEAD have such a name. Ownership over all history reads the aggregate for the files
that were never renamed and the rows for the rest. An aggregate keyed by the name at the
indexed tip, rebuilt when a new rename moves old rows, is the planned fix.
`crates/gitterrace-index/tests/latency.rs` holds today's times on a 100k-commit synthetic
history as ceilings, so the read path cannot get slower unnoticed.

## Not measured

Browser rendering and JavaScript, network latency (the server is local), concurrent clients,
the worktree view, incremental index updates after new commits, and a spinning disk.

## Reproduce

```sh
make bench          # control, git, rust. Needs the network and about 20 GB of disk
make bench LARGE=1  # adds linux
make bench-report   # rewrite the block below from the newest bench/results file
make gc BENCH=1     # print the path and size, then remove the clones (DRY=1 only prints)
```

`make check` runs only `bench-guard`, which drives the harness once against a tiny generated
repository to prove it still works.

## Results

The newest run in full. Every kept run, with its raw numbers and charts, is in
[bench/results](../bench/results/).

<!-- BEGIN GENERATED: results -->
Run 2026-11-03 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS 26.6.2 (arm64). git
version 2.54.0 (Apple Git-157), 0.1.0. 5 warm runs per page, 60
s idle windows. Every timed phase started with the 1-minute load average under 5.

- **control:** not measured on a quiet machine (other processes used 5 or more cores during
  export#3), so its numbers are withheld.
- **git:** not measured on a quiet machine (other processes used 5 or more cores during export#3,
  idle, index#2), so its numbers are withheld.
- **linux:** not measured on a quiet machine (other processes used 5 or more cores during export,
  idle, index), so its numbers are withheld.

### rust

large repository, measured at `f90a2d27c2`. All 5 metrics are within budget. Against budget: idle
CPU 0.10 % of 1 %, index build 89.5 s (86.6 to 93.6) of 2400 s, cold page 1710 ms (1647 to 1817) of
10000 ms, warm page 39 ms of 300 ms, peak memory 1166 MB of 6000 MB.

| Repo | Class | Measured at | Index | Peak RSS | Cache | RSS after pages | Idle footprint | Idle CPU | Idle CPU with /events | Export | Cold samples |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| rust | large | `f90a2d27c2` | 89.5 s | 1956 MB | 715 MB | 1166 MB | not measured | 0.03% | 0.10% | 361.7 s, 355 MB | median of 3 |

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-03-apple-m1-max/page_warm_ms-dark.svg">
<img alt="Warm page, slowest median per repository against its budget" src="../bench/results/2026-11-03-apple-m1-max/page_warm_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-03-apple-m1-max/page_cold_ms-dark.svg">
<img alt="Cold page, slowest per repository against its budget" src="../bench/results/2026-11-03-apple-m1-max/page_cold_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-03-apple-m1-max/index_secs-dark.svg">
<img alt="Index build per repository against its budget" src="../bench/results/2026-11-03-apple-m1-max/index_secs.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-03-apple-m1-max/idle_cpu_pct-dark.svg">
<img alt="Idle CPU with /events client per repository against its budget" src="../bench/results/2026-11-03-apple-m1-max/idle_cpu_pct.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-03-apple-m1-max/rss_mb-dark.svg">
<img alt="Peak memory (RSS) per repository against its budget" src="../bench/results/2026-11-03-apple-m1-max/rss_mb.svg">
</picture>

### Compared with 2026-11-01

The previous run, on Apple M1 Max, 10 cores, 64 GB RAM, macOS 26 (arm64). This run predates the
quiet-machine guard and recorded no load. Its charts stay beside its result file. On the charts
above, a hollow marker shows its value.

| Repo | Metric | 2026-11-01 | 2026-11-03 | Change |
| --- | --- | --- | --- | --- |
| rust | idle CPU | 0.15 % | 0.10 % | 33% less CPU |
| rust | index build | 334.8 s | 89.5 s | 73% faster |
| rust | cold page | 2099 ms | 1710 ms | 19% faster |
| rust | warm page | 398 ms | 39 ms | 90% faster |
| rust | peak memory | 2633 MB | 1166 MB | 56% less memory |

<details><summary>rust: every page</summary>

Probes: `library/stdarch/crates/core_arch/src/aarch64/neon/generated.rs` (largest text file), commit
`f90a2d27c2`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes | Previous warm |
| --- | --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 1.71 s | 35 ms | 39 ms | 38 ms | 47 ms | 9624164 | 1 ms |
| blob-largest | 10 ms | 8 ms | 9 ms | 9 ms | 10 ms | 1796773 | 1 ms |
| commit-largest | 47 ms | 1 ms | 1 ms | 1 ms | 1 ms | 71449 | 39 ms |
| diff-fragment | 29 ms | 30 ms | 30 ms | 29 ms | 31 ms | 12805 | 30 ms |
| log | 3 ms | 2 ms | 2 ms | 2 ms | 2 ms | 32574 | 2 ms |
| overview | 747 ms | 3 ms | 3 ms | 3 ms | 3 ms | 20826 | 398 ms |
| refs | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 6691 | 1 ms |
| search | 58 ms | 1 ms | 1 ms | 1 ms | 1 ms | 283295 | 53 ms |
| stats-overview | 697 ms | 1 ms | 1 ms | 1 ms | 1 ms | 29165 | 0 ms |
| stats/contributors | 158 ms | 1 ms | 1 ms | 1 ms | 1 ms | 76188 | 6 ms |
| stats/frequency | 207 ms | 1 ms | 1 ms | 1 ms | 1 ms | 17694 | 2 ms |
| stats/hotspots | 600 ms | 1 ms | 1 ms | 1 ms | 1 ms | 114853 | 1 ms |
| stats/languages | 858 ms | 1 ms | 1 ms | 1 ms | 1 ms | 384830 | 47 ms |
| stats/ownership | 635 ms | 1 ms | 1 ms | 1 ms | 1 ms | 20580 | 1 ms |
| stats/patterns | 1.07 s | 1 ms | 1 ms | 1 ms | 1 ms | 22547 | 3 ms |
| stats/types | 1.39 s | 1 ms | 1 ms | 1 ms | 2 ms | 45569 | 0 ms |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 7308 | 1 ms |
| tree-root | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 17706 | 2 ms |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 89.5 | 2400 | pass |
| page_cold_ms | 1709.8 | 10000 | pass |
| page_warm_ms | 39.3 | 300 | pass |
| rss_mb | 1165.6 | 6000 | pass |

</details>

<!-- END GENERATED: results -->
