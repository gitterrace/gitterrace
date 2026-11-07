# Benchmarks

gitterrace should feel instant on large repositories. This page says what is
measured, how, and what the latest run found. The harness is
`scripts/bench-run.py`, the corpus is `bench/corpus.toml`, and results land in
`bench/results/<date>-<machine>.json`, where the machine label comes from the
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
history. The list comes from `bench/corpus.toml`, each name links to its public
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

## Not measured

Browser rendering and JavaScript, network latency (the server is local),
concurrent clients, the worktree view, incremental index updates after new
commits, and a spinning disk.

## Reproduce

```sh
make bench          # control, git, rust. Needs the network and about 20 GB of disk
make bench LARGE=1  # adds linux
make bench-report   # rewrite the block below from the newest bench/results file
make gc BENCH=1     # print the path and size, then remove the clones (DRY=1 only prints)
```

`make check` runs only `bench-guard`, which drives the harness once against a
tiny generated repository to prove it still works.

## Results

The newest run in full. Every kept run, with its raw numbers and charts, is in
[bench/results](../bench/results/).

<!-- BEGIN GENERATED: results -->
Run 2026-11-03 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). git version 2.54.0 (Apple Git-157), gitterrace v0.1.0. 5 warm
runs per page, 60 s idle windows. Every timed phase started with the 1-minute
load average under 5.

- **rust:** not measured on a quiet machine (other processes used 5 or more
  cores during export, export#3, index), so its numbers are withheld.

### control

small repository, measured at the run of 2026-11-03. 6 of 7 metrics are within
budget. Against budget: export 12.3 s (12.0 to 12.3) of 60 s, idle footprint
above a fresh server 14 MB of 48 MB, idle CPU 1.03 % of 1 %, index build 0.2 s
(0.2 to 0.3) of 30 s, cold page 23 ms (22 to 24) of 1500 ms, warm page 11 ms of
300 ms, peak memory 40 MB of 500 MB.

Idle CPU is the larger of two windows: 0.05 % with no client and 1.03 % with one
/events client.

- **Over budget:** idle CPU is 1.03 % against a limit of 1 % (1.0x).

### git

medium repository, measured at `50fe5cc63f`. All 7 metrics are within budget.
Against budget: export 44.4 s (42.8 to 45.9) of 300 s, idle footprint above a
fresh server 28 MB of 48 MB, idle CPU 0.12 % of 1 %, index build 10.8 s (10.5 to
11.3) of 300 s, cold page 2355 ms (2321 to 2381) of 3000 ms, warm page 27 ms of
300 ms, peak memory 383 MB of 1500 MB.

Idle CPU is the larger of two windows: 0.07 % with no client and 0.12 % with one
/events client.

### linux

huge repository, measured at `cbbddb6b08`. 3 of 6 metrics are within budget.
Against budget: idle footprint above a fresh server 55 MB of 48 MB, idle CPU
0.17 % of 1 %, index build 1596.4 s (1596.4 to 1596.4) of 600 s, cold page 27674
ms (27674 to 27674) of 20000 ms, warm page 43 ms of 500 ms, peak memory 6449 MB
of 8000 MB.

Idle CPU is the larger of two windows: 0.10 % with no client and 0.17 % with one
/events client.

- **Over budget:** idle footprint above a fresh server is 55 MB against a limit
  of 48 MB (1.2x).
- **Over budget:** index build is 1596.4 s against a limit of 600 s (2.7x).
- **Over budget:** cold page is 27674 ms against a limit of 20000 ms (1.4x).

| Repo | Class | Measured at | Index | Peak RSS | Cache | RSS after pages | Idle footprint, total | Idle CPU | Idle CPU with /events | Export | Cold samples |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | the run of 2026-11-03 | 0.2 s | 37 MB | 0 MB | 40 MB | 23 MB | 0.05% | 1.03% | 12.3 s, 92 MB | median of 3 |
| git | medium | `50fe5cc63f` | 10.8 s | 672 MB | 69 MB | 383 MB | 37 MB | 0.07% | 0.12% | 44.4 s, 189 MB | median of 3 |
| linux | huge | `cbbddb6b08` | 1596.4 s | 8370 MB | 3595 MB | 6449 MB | 64 MB | 0.10% | 0.17% | failed (page cap reached) | 1 sample |
| rust | large | `f90a2d27c2` | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | median of 3 |

control is a working checkout, so with a tab open the server also checks it for
uncommitted changes every second. git, linux, rust are bare clones with no
working tree, so they have nothing to check and idle lower.

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

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-03-apple-m1-max/footprint_idle_mb-dark.svg">
<img alt="Idle footprint above a fresh server per repository against its budget" src="../bench/results/2026-11-03-apple-m1-max/footprint_idle_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="../bench/results/2026-11-03-apple-m1-max/export_secs-dark.svg">
<img alt="Export per repository against its budget" src="../bench/results/2026-11-03-apple-m1-max/export_secs.svg">
</picture>

<details><summary>control: every page</summary>

Probes: `site/assets/screenshots/blame-light.webp` (largest text file), commit
the run of 2026-11-03.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 6020 |
| blob-largest | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 6611 |
| commit-largest | 23 ms | 1 ms | 1 ms | 0 ms | 1 ms | 97470 |
| diff-fragment | 11 ms | 11 ms | 11 ms | 10 ms | 12 ms | 48789 |
| log | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 33672 |
| overview | 11 ms | 2 ms | 2 ms | 2 ms | 2 ms | 22538 |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 7764 |
| search | 7 ms | 0 ms | 1 ms | 0 ms | 1 ms | 321095 |
| stats-overview | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 26033 |
| stats/contributors | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 8234 |
| stats/frequency | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 11838 |
| stats/hotspots | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 108767 |
| stats/languages | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 127765 |
| stats/ownership | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 15391 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19152 |
| stats/types | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 37391 |
| tree-deep | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 7814 |
| tree-root | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 19496 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 12.3 | 60 | pass |
| footprint_idle_mb | 14.3 | 48 | pass |
| idle_cpu_pct | 1.0 | 1 | over |
| index_secs | 0.2 | 30 | pass |
| page_cold_ms | 23.2 | 1500 | pass |
| page_warm_ms | 10.9 | 300 | pass |
| rss_mb | 39.6 | 500 | pass |

</details>

<details><summary>git: every page</summary>

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 2.36 s | 24 ms | 27 ms | 26 ms | 28 ms | 7787360 |
| blob-largest | 10 ms | 8 ms | 9 ms | 9 ms | 10 ms | 1749049 |
| commit-largest | 5 ms | 1 ms | 1 ms | 1 ms | 1 ms | 10146 |
| diff-fragment | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 3461 |
| log | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 34111 |
| overview | 95 ms | 4 ms | 4 ms | 3 ms | 4 ms | 121493 |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 6749 |
| search | 20 ms | 1 ms | 1 ms | 1 ms | 1 ms | 222912 |
| stats-overview | 87 ms | 0 ms | 0 ms | 0 ms | 1 ms | 29906 |
| stats/contributors | 35 ms | 0 ms | 0 ms | 0 ms | 1 ms | 82058 |
| stats/frequency | 46 ms | 0 ms | 0 ms | 0 ms | 1 ms | 19999 |
| stats/hotspots | 55 ms | 0 ms | 0 ms | 0 ms | 1 ms | 94765 |
| stats/languages | 167 ms | 0 ms | 1 ms | 1 ms | 1 ms | 367183 |
| stats/ownership | 53 ms | 0 ms | 0 ms | 0 ms | 1 ms | 20618 |
| stats/patterns | 96 ms | 0 ms | 0 ms | 0 ms | 1 ms | 21334 |
| stats/types | 198 ms | 0 ms | 0 ms | 0 ms | 1 ms | 49269 |
| tree-deep | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 6527 |
| tree-root | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 118473 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 44.4 | 300 | pass |
| footprint_idle_mb | 28.5 | 48 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 10.8 | 300 | pass |
| page_cold_ms | 2355.3 | 3000 | pass |
| page_warm_ms | 27.5 | 300 | pass |
| rss_mb | 382.8 | 1500 | pass |

</details>

<details><summary>linux: every page</summary>

Probes: `drivers/gpu/drm/amd/include/asic_reg/nbio/nbio_6_1_default.h` (largest
text file), commit `cbbddb6b08`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 446 ms | 10 ms | 11 ms | 11 ms | 12 ms | 4395847 |
| blob-largest | 8 ms | 8 ms | 9 ms | 8 ms | 9 ms | 2773300 |
| commit-largest | 68 ms | 2 ms | 2 ms | 2 ms | 7 ms | 134956 |
| diff-fragment | 48 ms | 43 ms | 43 ms | 42 ms | 45 ms | 1345 |
| log | 4 ms | 3 ms | 3 ms | 3 ms | 4 ms | 35456 |
| overview | 3.26 s | 4 ms | 4 ms | 3 ms | 4 ms | 22047 |
| refs | 3 ms | 1 ms | 1 ms | 1 ms | 2 ms | 6784 |
| search | 256 ms | 1 ms | 1 ms | 1 ms | 1 ms | 309720 |
| stats-overview | 3.53 s | 1 ms | 1 ms | 0 ms | 2 ms | 36239 |
| stats/contributors | 767 ms | 1 ms | 1 ms | 1 ms | 1 ms | 117120 |
| stats/frequency | 1.02 s | 0 ms | 0 ms | 0 ms | 1 ms | 34321 |
| stats/hotspots | 2.33 s | 0 ms | 1 ms | 0 ms | 2 ms | 102301 |
| stats/languages | 27.67 s | 1 ms | 1 ms | 1 ms | 3 ms | 407201 |
| stats/ownership | 2.18 s | 0 ms | 0 ms | 0 ms | 2 ms | 20344 |
| stats/patterns | 4.20 s | 1 ms | 1 ms | 0 ms | 2 ms | 22614 |
| stats/types | 7.15 s | 0 ms | 0 ms | 0 ms | 2 ms | 50470 |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 16079 |
| tree-root | 3 ms | 2 ms | 2 ms | 2 ms | 3 ms | 18955 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| footprint_idle_mb | 55.4 | 48 | over |
| idle_cpu_pct | 0.2 | 1 | pass |
| index_secs | 1596.4 | 600 | over |
| page_cold_ms | 27673.9 | 20000 | over |
| page_warm_ms | 43.3 | 500 | pass |
| rss_mb | 6448.9 | 8000 | pass |

</details>

<!-- END GENERATED: results -->
