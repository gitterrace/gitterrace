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
Run 2026-11-01T20:11:08 on Apple M1 Max, 10 cores, 64.0 GB, Darwin 25.6.0. git version 2.54.0 (Apple Git-157), gitterrace 0.1.0. 5 warm runs per page, 60 s idle windows.

#### control

small repository, pinned at `d2ef86fee2`. All 6 metrics are within budget. Against budget: warm page 14 ms of 300 ms, cold page 25 ms of 1500 ms, index build 0.3 s of 30 s, idle CPU 0.57 % of 1 %, peak memory 56 MB of 500 MB, export 3.5 s of 60 s.

#### git

medium repository, pinned at `50fe5cc63f`. All 6 metrics are within budget. Against budget: warm page 85 ms of 300 ms, cold page 2476 ms of 3000 ms, index build 34.8 s of 300 s, idle CPU 0.10 % of 1 %, peak memory 1332 MB of 1500 MB, export 7.4 s of 300 s.

#### rust

large repository, pinned at `f90a2d27c2`. 4 of 5 metrics are within budget. Against budget: warm page 398 ms of 300 ms, cold page 2099 ms of 10000 ms, index build 334.8 s of 2400 s, idle CPU 0.15 % of 1 %, peak memory 2633 MB of 6000 MB.

- **Over budget:** warm page is 398 ms against a limit of 300 ms (1.3x).

| Repo | Class | Pin | Index | Peak RSS | Cache | RSS after pages | Idle CPU | Idle CPU with /events | Export |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | `d2ef86fee2` | 0.3 s | 53 MB | 0 MB | 56 MB | 0.05% | 0.57% | 3.5 s, 36 MB |
| git | medium | `50fe5cc63f` | 34.8 s | 1107 MB | 43 MB | 1332 MB | 0.03% | 0.10% | 7.4 s, 158 MB |
| rust | large | `f90a2d27c2` | 334.8 s | 2025 MB | 485 MB | 2633 MB | 0.03% | 0.15% | not run |

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Warm page, slowest median per repository against its budget">
<title>Warm page, slowest median (ms) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Warm page, slowest median (ms)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="12.8" height="18" rx="2" fill="#5f8a6e"/>
<line x1="348.9" y1="30" x2="348.9" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">14 of 300</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="75.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="348.9" y1="64" x2="348.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">85 of 300</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="over" x="84" y="102" width="351.9" height="18" rx="2" fill="#b5654a"/>
<line x1="348.9" y1="98" x2="348.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">398 of 300 (over)</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Cold page, slowest per repository against its budget">
<title>Cold page, slowest (ms) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Cold page, slowest (ms)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="1.5" height="18" rx="2" fill="#5f8a6e"/>
<line x1="136.8" y1="30" x2="136.8" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">25 of 1500</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="87.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="189.6" y1="64" x2="189.6" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">2476 of 3000</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="73.9" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">2099 of 10000</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Index build per repository against its budget">
<title>Index build (s) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Index build (s)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="1.5" height="18" rx="2" fill="#5f8a6e"/>
<line x1="88.4" y1="30" x2="88.4" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">0.3 of 30</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="5.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="128.0" y1="64" x2="128.0" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">34.8 of 300</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="49.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">334.8 of 2400</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Idle CPU with /events client per repository against its budget">
<title>Idle CPU with /events client (%) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Idle CPU with /events client (%)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="199.2" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="30" x2="435.9" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">0.57 of 1</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="35.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="64" x2="435.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">0.10 of 1</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="52.7" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">0.15 of 1</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Peak memory (RSS) per repository against its budget">
<title>Peak memory (RSS) (MB) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Peak memory (RSS) (MB)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="3.3" height="18" rx="2" fill="#5f8a6e"/>
<line x1="113.3" y1="30" x2="113.3" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">56 of 500</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="78.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="172.0" y1="64" x2="172.0" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">1332 of 1500</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="154.4" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">2633 of 6000</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 120" width="640" aria-label="Export per repository against its budget">
<title>Export (s) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Export (s)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="4.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="154.4" y1="30" x2="154.4" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">3.5 of 60</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="8.7" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="64" x2="435.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">7.4 of 300</text>
<text x="84" y="114" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<details><summary>control: every page</summary>

Probes: `crates/gitterrace-web/static/vendor/mermaid.min.js` (largest text file).

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 8 ms | 14 ms | 14 ms | 9 ms | 23 ms | 15262 |
| tree-root | 17 ms | 6 ms | 6 ms | 3 ms | 37 ms | 14606 |
| log | 6 ms | 7 ms | 7 ms | 4 ms | 8 ms | 32281 |
| refs | 9 ms | 11 ms | 11 ms | 4 ms | 15 ms | 6034 |
| commit-largest | 17 ms | 6 ms | 6 ms | 4 ms | 14 ms | 12350 |
| diff-fragment | 10 ms | 7 ms | 7 ms | 6 ms | 15 ms | 3506 |
| search | 25 ms | 13 ms | 13 ms | 6 ms | 31 ms | 355916 |
| tree-deep | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5797 |
| blob-largest | 1 ms | 1 ms | 1 ms | 1 ms | 15 ms | 5664 |
| blame-long-history | 9 ms | 5 ms | 5 ms | 2 ms | 6 ms | 5438 |
| stats-overview | 4 ms | 5 ms | 5 ms | 3 ms | 9 ms | 24604 |
| stats/types | 3 ms | 1 ms | 1 ms | 1 ms | 1 ms | 35728 |
| stats/languages | 9 ms | 13 ms | 13 ms | 7 ms | 17 ms | 197086 |
| stats/frequency | 1 ms | 2 ms | 2 ms | 1 ms | 2 ms | 9983 |
| stats/hotspots | 1 ms | 2 ms | 2 ms | 1 ms | 4 ms | 71358 |
| stats/ownership | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 12262 |
| stats/patterns | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 18018 |
| stats/contributors | 1 ms | 1 ms | 1 ms | 0 ms | 1 ms | 7141 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 14.4 | 300 | pass |
| page_cold_ms | 24.8 | 1500 | pass |
| index_secs | 0.3 | 30 | pass |
| idle_cpu_pct | 0.6 | 1 | pass |
| rss_mb | 56.2 | 500 | pass |
| export_secs | 3.5 | 60 | pass |

</details>

<details><summary>git: every page</summary>

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 228 ms | 51 ms | 51 ms | 46 ms | 66 ms | 110662 |
| tree-root | 4 ms | 4 ms | 4 ms | 4 ms | 4 ms | 110017 |
| log | 7 ms | 3 ms | 3 ms | 3 ms | 3 ms | 32181 |
| refs | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 6000 |
| commit-largest | 6 ms | 5 ms | 5 ms | 4 ms | 7 ms | 9475 |
| diff-fragment | 4 ms | 4 ms | 4 ms | 3 ms | 4 ms | 3461 |
| search | 16 ms | 14 ms | 14 ms | 14 ms | 18 ms | 366848 |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 5764 |
| blob-largest | 11 ms | 10 ms | 10 ms | 10 ms | 12 ms | 1464947 |
| blame-long-history | 2.48 s | 28 ms | 30 ms | 30 ms | 31 ms | 7241923 |
| stats-overview | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 28924 |
| stats/types | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 47541 |
| stats/languages | 85 ms | 85 ms | 85 ms | 85 ms | 90 ms | 360062 |
| stats/frequency | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 19109 |
| stats/hotspots | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 88619 |
| stats/ownership | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19187 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 19662 |
| stats/contributors | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 1736039 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 85.1 | 300 | pass |
| page_cold_ms | 2475.6 | 3000 | pass |
| index_secs | 34.8 | 300 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| rss_mb | 1332.2 | 1500 | pass |
| export_secs | 7.4 | 300 | pass |

</details>

<details><summary>rust: every page</summary>

Probes: `src/doc/rustc-dev-guide/src/queries/example-0.html` (largest text file), commit `f90a2d27c2`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 2.10 s | 398 ms | 398 ms | 379 ms | 427 ms | 17466 |
| tree-root | 34 ms | 2 ms | 2 ms | 2 ms | 32 ms | 16774 |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 3 ms | 31735 |
| refs | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 6034 |
| commit-largest | 47 ms | 39 ms | 39 ms | 35 ms | 40 ms | 70805 |
| diff-fragment | 30 ms | 30 ms | 30 ms | 30 ms | 30 ms | 12805 |
| search | 55 ms | 53 ms | 53 ms | 50 ms | 53 ms | 387119 |
| tree-deep | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 6629 |
| blob-largest | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5704 |
| blame-long-history | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5478 |
| stats-overview | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 28236 |
| stats/types | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 44838 |
| stats/languages | 48 ms | 47 ms | 47 ms | 47 ms | 49 ms | 380865 |
| stats/frequency | 2 ms | 2 ms | 2 ms | 1 ms | 3 ms | 16966 |
| stats/hotspots | 515 ms | 1 ms | 1 ms | 0 ms | 1 ms | 111518 |
| stats/ownership | 511 ms | 1 ms | 1 ms | 0 ms | 2 ms | 19562 |
| stats/patterns | 913 ms | 3 ms | 3 ms | 1 ms | 5 ms | 21397 |
| stats/contributors | 3 ms | 1 ms | 6 ms | 2 ms | 8 ms | 5241084 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 398.4 | 300 | over |
| page_cold_ms | 2099.2 | 10000 | pass |
| index_secs | 334.8 | 2400 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| rss_mb | 2633.0 | 6000 | pass |

</details>

<!-- END GENERATED: results -->
