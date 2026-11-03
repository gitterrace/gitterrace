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
4. **Memory.** RSS after the index is ready and after every page was visited.
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

Timings need a quiet machine. Before each timed phase the harness waits until the 1-minute load
average stays under `BENCH_MAX_LOAD` (default half the cores) for 20 s, and records the load per
phase. `make docs-check` refuses results timed above that limit.

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
Run 2026-11-01 on Apple M1 Max, 10 cores, 64 GB RAM, macOS 26 (arm64). git version 2.54.0 (Apple
Git-157), 0.1.0. 5 warm runs per page, 60 s idle windows. This run predates the quiet-machine guard
and recorded no load.

### control

small repository. All 6 metrics are within budget. Against budget: export
3.5 s of 60 s, idle CPU 0.57 % of 1 %, index build 0.3 s of 30 s, cold page 25 ms of 1500 ms, warm
page 14 ms of 300 ms, peak memory 56 MB of 500 MB.

### git

medium repository, measured at `50fe5cc63f`. All 6 metrics are within budget. Against budget: export
7.4 s of 300 s, idle CPU 0.10 % of 1 %, index build 34.8 s of 300 s, cold page 2476 ms of 3000 ms,
warm page 85 ms of 300 ms, peak memory 1332 MB of 1500 MB.

### rust

large repository, measured at `f90a2d27c2`. 4 of 5 metrics are within budget. Against budget: idle
CPU 0.15 % of 1 %, index build 334.8 s of 2400 s, cold page 2099 ms of 10000 ms, warm page 398 ms of
300 ms, peak memory 2633 MB of 6000 MB.

- **Over budget:** warm page is 398 ms against a limit of 300 ms (1.3x).

| Repo | Class | Measured at | Index | Peak RSS | Cache | RSS after pages | Idle CPU | Idle CPU with /events | Export | Cold samples |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | 0.1.0 | 0.3 s | 53 MB | 0 MB | 56 MB | 0.05% | 0.57% | 3.5 s, 36 MB | 1 sample, before repeats |
| git | medium | `50fe5cc63f` | 34.8 s | 1107 MB | 43 MB | 1332 MB | 0.03% | 0.10% | 7.4 s, 158 MB | 1 sample, before repeats |
| rust | large | `f90a2d27c2` | 334.8 s | 2025 MB | 485 MB | 2633 MB | 0.03% | 0.15% | not run | 1 sample, before repeats |

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-01-apple-m1-max/page_warm_ms-dark.svg">
<img alt="Warm page, slowest median per repository against its budget" src="../bench/results/2026-11-01-apple-m1-max/page_warm_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-01-apple-m1-max/page_cold_ms-dark.svg">
<img alt="Cold page, slowest per repository against its budget" src="../bench/results/2026-11-01-apple-m1-max/page_cold_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-01-apple-m1-max/index_secs-dark.svg">
<img alt="Index build per repository against its budget" src="../bench/results/2026-11-01-apple-m1-max/index_secs.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-01-apple-m1-max/idle_cpu_pct-dark.svg">
<img alt="Idle CPU with /events client per repository against its budget" src="../bench/results/2026-11-01-apple-m1-max/idle_cpu_pct.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-01-apple-m1-max/rss_mb-dark.svg">
<img alt="Peak memory (RSS) per repository against its budget" src="../bench/results/2026-11-01-apple-m1-max/rss_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-01-apple-m1-max/export_secs-dark.svg">
<img alt="Export per repository against its budget" src="../bench/results/2026-11-01-apple-m1-max/export_secs.svg">
</picture>

<details><summary>control: every page</summary>

Probes: `crates/gitterrace-web/static/vendor/mermaid.min.js` (largest text file).

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 9 ms | 5 ms | 5 ms | 2 ms | 6 ms | 5438 |
| blob-largest | 1 ms | 1 ms | 1 ms | 1 ms | 15 ms | 5664 |
| commit-largest | 17 ms | 6 ms | 6 ms | 4 ms | 14 ms | 12350 |
| diff-fragment | 10 ms | 7 ms | 7 ms | 6 ms | 15 ms | 3506 |
| log | 6 ms | 7 ms | 7 ms | 4 ms | 8 ms | 32281 |
| overview | 8 ms | 14 ms | 14 ms | 9 ms | 23 ms | 15262 |
| refs | 9 ms | 11 ms | 11 ms | 4 ms | 14 ms | 6034 |
| search | 25 ms | 13 ms | 13 ms | 6 ms | 31 ms | 355916 |
| stats-overview | 4 ms | 5 ms | 5 ms | 3 ms | 9 ms | 24604 |
| stats/contributors | 1 ms | 1 ms | 1 ms | 0 ms | 1 ms | 7141 |
| stats/frequency | 1 ms | 2 ms | 2 ms | 1 ms | 2 ms | 9983 |
| stats/hotspots | 1 ms | 2 ms | 2 ms | 1 ms | 4 ms | 71358 |
| stats/languages | 9 ms | 13 ms | 13 ms | 7 ms | 17 ms | 197086 |
| stats/ownership | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 12262 |
| stats/patterns | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 18018 |
| stats/types | 3 ms | 1 ms | 1 ms | 1 ms | 1 ms | 35728 |
| tree-deep | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5797 |
| tree-root | 17 ms | 6 ms | 6 ms | 3 ms | 37 ms | 14606 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 3.5 | 60 | pass |
| idle_cpu_pct | 0.6 | 1 | pass |
| index_secs | 0.3 | 30 | pass |
| page_cold_ms | 24.9 | 1500 | pass |
| page_warm_ms | 14.4 | 300 | pass |
| rss_mb | 56.2 | 500 | pass |

</details>

<details><summary>git: every page</summary>

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 2.48 s | 28 ms | 30 ms | 30 ms | 31 ms | 7241923 |
| blob-largest | 11 ms | 10 ms | 10 ms | 10 ms | 12 ms | 1464947 |
| commit-largest | 6 ms | 5 ms | 5 ms | 4 ms | 7 ms | 9475 |
| diff-fragment | 4 ms | 4 ms | 4 ms | 3 ms | 4 ms | 3461 |
| log | 7 ms | 3 ms | 3 ms | 3 ms | 3 ms | 32181 |
| overview | 228 ms | 51 ms | 51 ms | 46 ms | 66 ms | 110662 |
| refs | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 6000 |
| search | 16 ms | 14 ms | 14 ms | 14 ms | 18 ms | 366848 |
| stats-overview | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 28924 |
| stats/contributors | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 1736039 |
| stats/frequency | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 19109 |
| stats/hotspots | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 88619 |
| stats/languages | 85 ms | 85 ms | 85 ms | 85 ms | 90 ms | 360062 |
| stats/ownership | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19187 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 19662 |
| stats/types | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 47541 |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 5764 |
| tree-root | 4 ms | 4 ms | 4 ms | 4 ms | 4 ms | 110017 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 7.4 | 300 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 34.8 | 300 | pass |
| page_cold_ms | 2475.6 | 3000 | pass |
| page_warm_ms | 85.1 | 300 | pass |
| rss_mb | 1332.2 | 1500 | pass |

</details>

<details><summary>rust: every page</summary>

Probes: `src/doc/rustc-dev-guide/src/queries/example-0.html` (largest text file), commit
`f90a2d27c2`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5478 |
| blob-largest | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5704 |
| commit-largest | 47 ms | 39 ms | 39 ms | 35 ms | 40 ms | 70805 |
| diff-fragment | 30 ms | 30 ms | 30 ms | 30 ms | 30 ms | 12805 |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 3 ms | 31735 |
| overview | 2.10 s | 398 ms | 398 ms | 379 ms | 427 ms | 17466 |
| refs | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 6034 |
| search | 55 ms | 53 ms | 53 ms | 50 ms | 53 ms | 387119 |
| stats-overview | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 28236 |
| stats/contributors | 3 ms | 1 ms | 6 ms | 2 ms | 8 ms | 5241084 |
| stats/frequency | 2 ms | 2 ms | 2 ms | 1 ms | 3 ms | 16966 |
| stats/hotspots | 515 ms | 1 ms | 1 ms | 0 ms | 1 ms | 111518 |
| stats/languages | 48 ms | 47 ms | 47 ms | 47 ms | 49 ms | 380865 |
| stats/ownership | 511 ms | 1 ms | 1 ms | 0 ms | 2 ms | 19562 |
| stats/patterns | 913 ms | 3 ms | 3 ms | 1 ms | 5 ms | 21397 |
| stats/types | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 44838 |
| tree-deep | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 6629 |
| tree-root | 34 ms | 2 ms | 2 ms | 2 ms | 32 ms | 16774 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 334.8 | 2400 | pass |
| page_cold_ms | 2099.2 | 10000 | pass |
| page_warm_ms | 398.4 | 300 | over |
| rss_mb | 2632.9 | 6000 | pass |

</details>

<!-- END GENERATED: results -->
