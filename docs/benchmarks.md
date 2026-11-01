# Benchmarks

gitterrace should feel instant on very large repositories. This page says what is measured,
how, and what the latest run found. The harness is `scripts/bench-run.py`, the corpus is
`bench/corpus.toml`, and results land in `bench/results/`.

## What is measured

Release build only. Every page metric is the median of N warm runs (default 5) with min and
max. Cold is the first request after the index is ready, with the OS file cache warmed by one
untimed read of the repository. Warm is the runs after it.

1. **Index build.** Seconds from process start until `/api/progress` reports ready, from an
   empty cache, plus peak RSS sampled meanwhile and the size of the cache on disk.
2. **Pages.** Time to first byte and full response for `/`, `/tree/HEAD`, a deep tree path,
   `/blob` of the largest text file, `/blame` of that file, `/log/HEAD`, `/commit` and
   `/diff` fragment of the newest commit, `/search` for a common identifier, `/refs`, and
   every `/stats` page. The probes are discovered from the pinned tree.
3. **Idle.** CPU percent over a 60 s window with no client, and again with one `/events`
   client open, which is the cost of the live watcher.
4. **Memory.** RSS after the index is ready and after every page was visited.
5. **Export.** `git terrace export` time and output size, on the small and medium repos only.

## Why these repositories

Each is pinned to a sha in the corpus file, so a re-run measures the same history.

- **control**: this repository, the floor.
- **git/git**: medium, long-lived, a classic reference.
- **rust-lang/rust**: very large history, many files and contributors.
- **torvalds/linux**: the extreme, opt-in with `BENCH_LARGE=1` (`make bench-large`). It needs
  about 30 GB and a long first index. Not yet run.

Clones are bare, blobless fetches of the pinned sha followed by `git backfill`, kept in
`$GITTERRACE_BENCH_DIR` (default `~/.cache/gitterrace-bench`), never in the repository. The
harness refuses to clone when free disk is under the corpus `need_gb`.

## Budgets

Each size class has a target per metric (warm page, cold page, index time, idle CPU, RSS,
export) in the corpus file. They come from the first measured run plus headroom, except the
warm page target of 300 ms and the idle target of 1% CPU, which are product goals. A run
marks each metric pass or over. Over budget does not fail the run unless `BENCH_STRICT=1`.

## Not measured

Browser rendering and JavaScript, network latency (the server is local), concurrent clients,
the worktree view, incremental index updates after new commits, and a spinning disk.

## Reproduce

```
make bench          # control, git, rust. Needs the network and about 20 GB of disk
make bench-large    # adds linux
make bench-report   # rewrite the block below from the newest bench/results file
make bench-clean    # print the path and size, then remove the clones
```

`make check` runs only `bench-guard`, which drives the harness once against a tiny generated
repository to prove it still works.

## Results

<!-- BEGIN GENERATED: results -->
Run 2026-11-01T14:13:10 on Apple M1 Max, 10 cores, 64.0 GB, Darwin 25.6.0. git version 2.54.0 (Apple Git-157), gitterrace 0.1.0. 5 warm runs per page, 60 s idle windows.

| Repo | Class | Pin | Index | Peak RSS | Cache | RSS after pages | Idle CPU | Idle CPU with /events | Export |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | `d2ef86fee2` | 0.5 s | 50 MB | 0 MB | 52 MB | 0.03% | 0.40% | 2.1 s, 26 MB |
| git | medium | `50fe5cc63f` | 58.4 s | 756 MB | 47 MB | 1384 MB | 0.05% | 0.47% | 7.4 s, 153 MB |
| rust | large | `f90a2d27c2` | 1262.3 s | 1981 MB | 685 MB | 4837 MB | 1.03% | 4.71% | not run |

#### control

Probes: `crates/gitterrace-web/static/vendor/mermaid.min.js` (largest text file).

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 14 ms | 7 ms | 7 ms | 7 ms | 15 ms | 12294 |
| tree-root | 37 ms | 8 ms | 8 ms | 3 ms | 14 ms | 11723 |
| log | 5 ms | 4 ms | 5 ms | 4 ms | 7 ms | 27940 |
| refs | 8 ms | 5 ms | 5 ms | 4 ms | 14 ms | 5523 |
| commit-largest | 11 ms | 6 ms | 6 ms | 4 ms | 7 ms | 22741 |
| diff-fragment | 6 ms | 4 ms | 4 ms | 4 ms | 5 ms | 14094 |
| search | 10 ms | 9 ms | 9 ms | 8 ms | 10 ms | 358152 |
| tree-deep | 4 ms | 4 ms | 4 ms | 4 ms | 5 ms | 5598 |
| blob-largest | 2 ms | 3 ms | 3 ms | 2 ms | 3 ms | 5465 |
| blame-long-history | 3 ms | 3 ms | 3 ms | 2 ms | 4 ms | 5239 |
| stats-overview | 3 ms | 6 ms | 6 ms | 4 ms | 9 ms | 25308 |
| stats/files | 4 ms | 1 ms | 1 ms | 1 ms | 2 ms | 22837 |
| stats/types | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 32853 |
| stats/languages | 2 ms | 3 ms | 3 ms | 2 ms | 4 ms | 137119 |
| stats/frequency | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 9450 |
| stats/pulse | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 13292 |
| stats/hotspots | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 66567 |
| stats/ownership | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 10898 |
| stats/patterns | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 17786 |
| stats/contributors | 2 ms | 2 ms | 2 ms | 1 ms | 2 ms | 6864 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 9.2 | 300 | pass |
| page_cold_ms | 36.9 | 1500 | pass |
| index_secs | 0.5 | 30 | pass |
| idle_cpu_pct | 0.4 | 1 | pass |
| rss_mb | 52.0 | 500 | pass |
| export_secs | 2.1 | 60 | pass |

#### git

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 230 ms | 18 ms | 18 ms | 16 ms | 19 ms | 110166 |
| tree-root | 4 ms | 3 ms | 3 ms | 3 ms | 4 ms | 109598 |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 27918 |
| refs | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5498 |
| commit-largest | 5 ms | 4 ms | 4 ms | 3 ms | 4 ms | 9165 |
| diff-fragment | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 3461 |
| search | 15 ms | 14 ms | 14 ms | 13 ms | 15 ms | 378692 |
| tree-deep | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 5558 |
| blob-largest | 10 ms | 9 ms | 9 ms | 9 ms | 10 ms | 1464741 |
| blame-long-history | 2.46 s | 25 ms | 27 ms | 25 ms | 29 ms | 6483765 |
| stats-overview | 19 ms | 19 ms | 19 ms | 19 ms | 20 ms | 38345 |
| stats/files | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 21665 |
| stats/types | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 45398 |
| stats/languages | 3 ms | 3 ms | 3 ms | 3 ms | 4 ms | 357911 |
| stats/frequency | 1 ms | 1 ms | 1 ms | 0 ms | 1 ms | 18942 |
| stats/pulse | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 13433 |
| stats/hotspots | 1 ms | 1 ms | 1 ms | 0 ms | 1 ms | 92736 |
| stats/ownership | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 18888 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19450 |
| stats/contributors | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 1735756 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 27.4 | 300 | pass |
| page_cold_ms | 2459.3 | 3000 | pass |
| index_secs | 58.4 | 300 | pass |
| idle_cpu_pct | 0.5 | 1 | pass |
| rss_mb | 1384.4 | 1500 | pass |
| export_secs | 7.4 | 300 | pass |

#### rust

Probes: `src/doc/rustc-dev-guide/src/queries/example-0.html` (largest text file), commit `f90a2d27c2`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 2.71 s | 291 ms | 291 ms | 262 ms | 327 ms | 16995 |
| tree-root | 3 ms | 3 ms | 3 ms | 2 ms | 6 ms | 16391 |
| log | 7 ms | 4 ms | 4 ms | 4 ms | 5 ms | 27491 |
| refs | 3 ms | 2 ms | 2 ms | 2 ms | 3 ms | 5531 |
| commit-largest | 50 ms | 40 ms | 40 ms | 39 ms | 54 ms | 70444 |
| diff-fragment | 70 ms | 36 ms | 36 ms | 34 ms | 41 ms | 12805 |
| search | 137 ms | 72 ms | 72 ms | 69 ms | 76 ms | 292733 |
| tree-deep | 6 ms | 8 ms | 8 ms | 5 ms | 11 ms | 6423 |
| blob-largest | 3 ms | 3 ms | 3 ms | 2 ms | 3 ms | 5498 |
| blame-long-history | 3 ms | 3 ms | 3 ms | 2 ms | 4 ms | 5272 |
| stats-overview | 105 ms | 100 ms | 100 ms | 97 ms | 103 ms | 37983 |
| stats/files | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 24174 |
| stats/types | 2.27 s | 1 ms | 1 ms | 1 ms | 1 ms | 43150 |
| stats/languages | 4 ms | 4 ms | 4 ms | 4 ms | 4 ms | 392569 |
| stats/frequency | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 16769 |
| stats/pulse | 186 ms | 1 ms | 1 ms | 1 ms | 1 ms | 13372 |
| stats/hotspots | 1.49 s | 0 ms | 0 ms | 0 ms | 49 ms | 115635 |
| stats/ownership | 758 ms | 1 ms | 1 ms | 1 ms | 1 ms | 19263 |
| stats/patterns | 1.84 s | 1 ms | 1 ms | 1 ms | 54 ms | 21161 |
| stats/contributors | 3 ms | 1 ms | 3 ms | 2 ms | 6 ms | 5240801 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 290.7 | 300 | pass |
| page_cold_ms | 2714.4 | 10000 | pass |
| index_secs | 1262.3 | 1800 | pass |
| idle_cpu_pct | 4.7 | 1 | over |
| rss_mb | 4837.0 | 4000 | over |

<!-- END GENERATED: results -->
