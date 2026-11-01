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
Run 2026-11-01T15:58:55 on Apple M1 Max, 10 cores, 64.0 GB, Darwin 25.6.0. git version 2.54.0 (Apple Git-157), gitterrace 0.1.0. 5 warm runs per page, 60 s idle windows.

#### control

small repository, pinned at `d2ef86fee2`. All 6 metrics are within budget. Against budget: warm page 9 ms of 300 ms, cold page 19 ms of 1500 ms, index build 0.2 s of 30 s, idle CPU 0.73 % of 1 %, peak memory 48 MB of 500 MB, export 1.9 s of 60 s.

#### git

medium repository, pinned at `50fe5cc63f`. All 6 metrics are within budget. Against budget: warm page 30 ms of 300 ms, cold page 2376 ms of 3000 ms, index build 27.0 s of 300 s, idle CPU 0.48 % of 1 %, peak memory 1487 MB of 1500 MB, export 6.5 s of 300 s.

#### rust

large repository, pinned at `f90a2d27c2`. 4 of 5 metrics are within budget. Against budget: warm page 163 ms of 300 ms, cold page 6566 ms of 10000 ms, index build 276.7 s of 2400 s, idle CPU 4.71 % of 1 %, peak memory 2543 MB of 6000 MB.

- **Over budget:** idle CPU is 4.71 % against a limit of 1 % (4.7x).

| Repo | Class | Pin | Index | Peak RSS | Cache | RSS after pages | Idle CPU | Idle CPU with /events | Export |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | `d2ef86fee2` | 0.2 s | 46 MB | 0 MB | 48 MB | 0.05% | 0.73% | 1.9 s, 29 MB |
| git | medium | `50fe5cc63f` | 27.0 s | 1149 MB | 43 MB | 1487 MB | 0.07% | 0.48% | 6.5 s, 154 MB |
| rust | large | `f90a2d27c2` | 276.7 s | 2027 MB | 485 MB | 2543 MB | 0.07% | 4.71% | not run |

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Warm page, slowest median per repository against its budget">
<title>Warm page, slowest median (ms) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Warm page, slowest median (ms)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="10.6" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="30" x2="435.9" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">9 of 300</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="35.0" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="64" x2="435.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">30 of 300</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="191.2" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">163 of 300</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Cold page, slowest per repository against its budget">
<title>Cold page, slowest (ms) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Cold page, slowest (ms)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="1.5" height="18" rx="2" fill="#5f8a6e"/>
<line x1="136.8" y1="30" x2="136.8" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">19 of 1500</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="83.6" height="18" rx="2" fill="#5f8a6e"/>
<line x1="189.6" y1="64" x2="189.6" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">2376 of 3000</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="231.0" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">6566 of 10000</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Index build per repository against its budget">
<title>Index build (s) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Index build (s)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="1.5" height="18" rx="2" fill="#5f8a6e"/>
<line x1="88.4" y1="30" x2="88.4" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">0.2 of 30</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="4.0" height="18" rx="2" fill="#5f8a6e"/>
<line x1="128.0" y1="64" x2="128.0" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">27.0 of 300</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="40.6" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">276.7 of 2400</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Idle CPU with /events client per repository against its budget">
<title>Idle CPU with /events client (%) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Idle CPU with /events client (%)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="54.7" height="18" rx="2" fill="#5f8a6e"/>
<line x1="158.6" y1="30" x2="158.6" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">0.73 of 1</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="36.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="158.6" y1="64" x2="158.6" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">0.48 of 1</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="over" x="84" y="102" width="351.9" height="18" rx="2" fill="#b5654a"/>
<line x1="158.6" y1="98" x2="158.6" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">4.71 of 1 (over)</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Peak memory (RSS) per repository against its budget">
<title>Peak memory (RSS) (MB) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Peak memory (RSS) (MB)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="2.8" height="18" rx="2" fill="#5f8a6e"/>
<line x1="113.3" y1="30" x2="113.3" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">48 of 500</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="87.2" height="18" rx="2" fill="#5f8a6e"/>
<line x1="172.0" y1="64" x2="172.0" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">1487 of 1500</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="149.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">2543 of 6000</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 120" width="640" aria-label="Export per repository against its budget">
<title>Export (s) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Export (s)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="2.3" height="18" rx="2" fill="#5f8a6e"/>
<line x1="154.4" y1="30" x2="154.4" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">1.9 of 60</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="7.6" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="64" x2="435.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">6.5 of 300</text>
<text x="84" y="114" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<details><summary>control: every page</summary>

Probes: `crates/gitterrace-web/static/vendor/mermaid.min.js` (largest text file).

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 3 ms | 3 ms | 3 ms | 2 ms | 6 ms | 13062 |
| tree-root | 3 ms | 2 ms | 2 ms | 2 ms | 2 ms | 12351 |
| log | 3 ms | 2 ms | 2 ms | 2 ms | 12 ms | 31195 |
| refs | 8 ms | 4 ms | 4 ms | 3 ms | 5 ms | 5915 |
| commit-largest | 7 ms | 7 ms | 7 ms | 4 ms | 7 ms | 19427 |
| diff-fragment | 5 ms | 9 ms | 9 ms | 8 ms | 13 ms | 13232 |
| search | 19 ms | 8 ms | 9 ms | 6 ms | 20 ms | 372822 |
| tree-deep | 2 ms | 2 ms | 2 ms | 2 ms | 3 ms | 5711 |
| blob-largest | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5578 |
| blame-long-history | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5352 |
| stats-overview | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 24051 |
| stats/files | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 23080 |
| stats/types | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 34505 |
| stats/languages | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 164285 |
| stats/frequency | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 9632 |
| stats/pulse | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 13394 |
| stats/hotspots | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 67112 |
| stats/ownership | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 11509 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 17937 |
| stats/contributors | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 7057 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 9.0 | 300 | pass |
| page_cold_ms | 19.2 | 1500 | pass |
| index_secs | 0.2 | 30 | pass |
| idle_cpu_pct | 0.7 | 1 | pass |
| rss_mb | 48.4 | 500 | pass |
| export_secs | 1.9 | 60 | pass |

</details>

<details><summary>git: every page</summary>

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 71 ms | 16 ms | 16 ms | 14 ms | 16 ms | 110386 |
| tree-root | 4 ms | 3 ms | 3 ms | 3 ms | 3 ms | 109704 |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 31431 |
| refs | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 5857 |
| commit-largest | 5 ms | 3 ms | 3 ms | 3 ms | 4 ms | 9375 |
| diff-fragment | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 3461 |
| search | 16 ms | 13 ms | 13 ms | 13 ms | 14 ms | 378799 |
| tree-deep | 3 ms | 2 ms | 2 ms | 2 ms | 2 ms | 5664 |
| blob-largest | 9 ms | 9 ms | 10 ms | 9 ms | 10 ms | 1464847 |
| blame-long-history | 2.38 s | 27 ms | 30 ms | 29 ms | 31 ms | 7241823 |
| stats-overview | 15 ms | 15 ms | 15 ms | 15 ms | 16 ms | 37019 |
| stats/files | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 21838 |
| stats/types | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 46874 |
| stats/languages | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 383969 |
| stats/frequency | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 18995 |
| stats/pulse | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 13528 |
| stats/hotspots | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 88521 |
| stats/ownership | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19073 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19548 |
| stats/contributors | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 1735941 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 29.8 | 300 | pass |
| page_cold_ms | 2376.2 | 3000 | pass |
| index_secs | 27.0 | 300 | pass |
| idle_cpu_pct | 0.5 | 1 | pass |
| rss_mb | 1487.2 | 1500 | pass |
| export_secs | 6.5 | 300 | pass |

</details>

<details><summary>rust: every page</summary>

Probes: `src/doc/rustc-dev-guide/src/queries/example-0.html` (largest text file), commit `f90a2d27c2`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 1.43 s | 163 ms | 163 ms | 158 ms | 169 ms | 17215 |
| tree-root | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 16497 |
| log | 3 ms | 3 ms | 3 ms | 2 ms | 3 ms | 31008 |
| refs | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5890 |
| commit-largest | 47 ms | 36 ms | 36 ms | 36 ms | 38 ms | 70705 |
| diff-fragment | 29 ms | 29 ms | 29 ms | 29 ms | 29 ms | 12805 |
| search | 55 ms | 56 ms | 56 ms | 55 ms | 61 ms | 387019 |
| tree-deep | 6 ms | 5 ms | 5 ms | 4 ms | 6 ms | 6529 |
| blob-largest | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 5604 |
| blame-long-history | 2 ms | 2 ms | 2 ms | 2 ms | 3 ms | 5378 |
| stats-overview | 80 ms | 78 ms | 78 ms | 78 ms | 127 ms | 36672 |
| stats/files | 6.57 s | 0 ms | 0 ms | 0 ms | 1 ms | 24347 |
| stats/types | 6.29 s | 0 ms | 0 ms | 0 ms | 1 ms | 44685 |
| stats/languages | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 418627 |
| stats/frequency | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 16852 |
| stats/pulse | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 13467 |
| stats/hotspots | 1.26 s | 0 ms | 0 ms | 0 ms | 1 ms | 111420 |
| stats/ownership | 512 ms | 0 ms | 0 ms | 0 ms | 1 ms | 19448 |
| stats/patterns | 1.39 s | 0 ms | 0 ms | 0 ms | 48 ms | 21283 |
| stats/contributors | 2 ms | 1 ms | 2 ms | 2 ms | 2 ms | 5240986 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 163.0 | 300 | pass |
| page_cold_ms | 6566.3 | 10000 | pass |
| index_secs | 276.7 | 2400 | pass |
| idle_cpu_pct | 4.7 | 1 | over |
| rss_mb | 2543.1 | 6000 | pass |

</details>

<!-- END GENERATED: results -->
