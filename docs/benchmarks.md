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
Run 2026-11-01T17:53:09 on Apple M1 Max, 10 cores, 64.0 GB, Darwin 25.6.0. git version 2.54.0 (Apple Git-157), gitterrace 0.1.0. 5 warm runs per page, 60 s idle windows.

#### control

small repository, pinned at `d2ef86fee2`. All 6 metrics are within budget. Against budget: warm page 13 ms of 300 ms, cold page 47 ms of 1500 ms, index build 0.2 s of 30 s, idle CPU 0.93 % of 1 %, peak memory 49 MB of 500 MB, export 2.2 s of 60 s.

#### git

medium repository, pinned at `50fe5cc63f`. 5 of 6 metrics are within budget. Against budget: warm page 29 ms of 300 ms, cold page 2359 ms of 3000 ms, index build 26.8 s of 300 s, idle CPU 0.10 % of 1 %, peak memory 1509 MB of 1500 MB, export 5.6 s of 300 s.

- **Over budget:** peak memory is 1509 MB against a limit of 1500 MB (1.0x).

#### rust

large repository, pinned at `f90a2d27c2`. All 5 metrics are within budget. Against budget: warm page 182 ms of 300 ms, cold page 3920 ms of 10000 ms, index build 273.8 s of 2400 s, idle CPU 0.22 % of 1 %, peak memory 2768 MB of 6000 MB.

| Repo | Class | Pin | Index | Peak RSS | Cache | RSS after pages | Idle CPU | Idle CPU with /events | Export |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | `d2ef86fee2` | 0.2 s | 45 MB | 0 MB | 49 MB | 0.07% | 0.93% | 2.2 s, 31 MB |
| git | medium | `50fe5cc63f` | 26.8 s | 1189 MB | 43 MB | 1509 MB | 0.07% | 0.10% | 5.6 s, 154 MB |
| rust | large | `f90a2d27c2` | 273.8 s | 1959 MB | 485 MB | 2768 MB | 0.22% | 0.12% | not run |

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Warm page, slowest median per repository against its budget">
<title>Warm page, slowest median (ms) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Warm page, slowest median (ms)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="14.8" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="30" x2="435.9" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">13 of 300</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="34.3" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="64" x2="435.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">29 of 300</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="212.9" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">182 of 300</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Cold page, slowest per repository against its budget">
<title>Cold page, slowest (ms) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Cold page, slowest (ms)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="1.7" height="18" rx="2" fill="#5f8a6e"/>
<line x1="136.8" y1="30" x2="136.8" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">47 of 1500</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="83.0" height="18" rx="2" fill="#5f8a6e"/>
<line x1="189.6" y1="64" x2="189.6" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">2359 of 3000</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="137.9" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">3920 of 10000</text>
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
<rect class="ok" x="84" y="68" width="3.9" height="18" rx="2" fill="#5f8a6e"/>
<line x1="128.0" y1="64" x2="128.0" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">26.8 of 300</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="40.1" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">273.8 of 2400</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Idle CPU with /events client per repository against its budget">
<title>Idle CPU with /events client (%) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Idle CPU with /events client (%)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="328.2" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="30" x2="435.9" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">0.93 of 1</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="35.2" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="64" x2="435.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">0.10 of 1</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="76.2" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">0.22 of 1</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 154" width="640" aria-label="Peak memory (RSS) per repository against its budget">
<title>Peak memory (RSS) (MB) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Peak memory (RSS) (MB)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="2.9" height="18" rx="2" fill="#5f8a6e"/>
<line x1="113.3" y1="30" x2="113.3" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">49 of 500</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="over" x="84" y="68" width="88.5" height="18" rx="2" fill="#b5654a"/>
<line x1="172.0" y1="64" x2="172.0" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">1509 of 1500 (over)</text>
<text x="8" y="115" fill="currentColor" font-family="sans-serif" font-size="12">rust</text>
<rect class="ok" x="84" y="102" width="162.3" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="98" x2="435.9" y2="124" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="115" fill="currentColor" font-family="sans-serif" font-size="12">2768 of 6000</text>
<text x="84" y="148" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 640 120" width="640" aria-label="Export per repository against its budget">
<title>Export (s) against budget</title>
<style>.ok{fill:#5f8a6e}.over{fill:#b5654a}@media(prefers-color-scheme:dark){.ok{fill:#8fb89c}.over{fill:#d98b72}}</style>
<text x="8" y="16" fill="currentColor" font-family="sans-serif" font-size="13" font-weight="600">Export (s)</text>
<text x="8" y="47" fill="currentColor" font-family="sans-serif" font-size="12">control</text>
<rect class="ok" x="84" y="34" width="2.6" height="18" rx="2" fill="#5f8a6e"/>
<line x1="154.4" y1="30" x2="154.4" y2="56" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="47" fill="currentColor" font-family="sans-serif" font-size="12">2.2 of 60</text>
<text x="8" y="81" fill="currentColor" font-family="sans-serif" font-size="12">git</text>
<rect class="ok" x="84" y="68" width="6.6" height="18" rx="2" fill="#5f8a6e"/>
<line x1="435.9" y1="64" x2="435.9" y2="90" stroke="currentColor" stroke-width="1.5" stroke-dasharray="3 2"/>
<text x="478" y="81" fill="currentColor" font-family="sans-serif" font-size="12">5.6 of 300</text>
<text x="84" y="114" fill="currentColor" font-family="sans-serif" font-size="11" opacity="0.7">dashed line: budget</text></svg>

<details><summary>control: every page</summary>

Probes: `crates/gitterrace-web/static/vendor/mermaid.min.js` (largest text file).

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 3 ms | 3 ms | 3 ms | 2 ms | 5 ms | 14322 |
| tree-root | 3 ms | 5 ms | 5 ms | 3 ms | 7 ms | 13596 |
| log | 7 ms | 3 ms | 3 ms | 3 ms | 4 ms | 31702 |
| refs | 1 ms | 4 ms | 4 ms | 1 ms | 5 ms | 5972 |
| commit-largest | 47 ms | 13 ms | 13 ms | 12 ms | 16 ms | 117706 |
| diff-fragment | 6 ms | 6 ms | 6 ms | 6 ms | 6 ms | 45538 |
| search | 7 ms | 5 ms | 5 ms | 5 ms | 6 ms | 384272 |
| tree-deep | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 5710 |
| blob-largest | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5577 |
| blame-long-history | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 5351 |
| stats-overview | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 24175 |
| stats/files | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 23121 |
| stats/types | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 34878 |
| stats/languages | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 169535 |
| stats/frequency | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 9631 |
| stats/pulse | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 13516 |
| stats/hotspots | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 68969 |
| stats/ownership | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 11730 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 17981 |
| stats/contributors | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 7056 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 12.6 | 300 | pass |
| page_cold_ms | 47.4 | 1500 | pass |
| index_secs | 0.2 | 30 | pass |
| idle_cpu_pct | 0.9 | 1 | pass |
| rss_mb | 48.7 | 500 | pass |
| export_secs | 2.2 | 60 | pass |

</details>

<details><summary>git: every page</summary>

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 27 ms | 15 ms | 15 ms | 14 ms | 17 ms | 110393 |
| tree-root | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 109711 |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 32088 |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5907 |
| commit-largest | 5 ms | 3 ms | 3 ms | 3 ms | 3 ms | 9382 |
| diff-fragment | 3 ms | 3 ms | 3 ms | 2 ms | 3 ms | 3461 |
| search | 12 ms | 11 ms | 11 ms | 11 ms | 11 ms | 366753 |
| tree-deep | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 5671 |
| blob-largest | 10 ms | 9 ms | 9 ms | 9 ms | 10 ms | 1464854 |
| blame-long-history | 2.36 s | 26 ms | 29 ms | 29 ms | 30 ms | 7241830 |
| stats-overview | 17 ms | 15 ms | 15 ms | 15 ms | 17 ms | 37026 |
| stats/files | 1 ms | 0 ms | 0 ms | 0 ms | 0 ms | 21845 |
| stats/types | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 47141 |
| stats/languages | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 359971 |
| stats/frequency | 0 ms | 0 ms | 0 ms | 0 ms | 1 ms | 19002 |
| stats/pulse | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 13535 |
| stats/hotspots | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 88528 |
| stats/ownership | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19080 |
| stats/patterns | 0 ms | 0 ms | 0 ms | 0 ms | 0 ms | 19555 |
| stats/contributors | 1 ms | 0 ms | 1 ms | 1 ms | 1 ms | 1735948 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 29.3 | 300 | pass |
| page_cold_ms | 2358.9 | 3000 | pass |
| index_secs | 26.8 | 300 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| rss_mb | 1509.3 | 1500 | over |
| export_secs | 5.6 | 300 | pass |

</details>

<details><summary>rust: every page</summary>

Probes: `src/doc/rustc-dev-guide/src/queries/example-0.html` (largest text file), commit `f90a2d27c2`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| overview | 1.53 s | 182 ms | 182 ms | 174 ms | 187 ms | 17222 |
| tree-root | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 16504 |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 3 ms | 31656 |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 5940 |
| commit-largest | 45 ms | 37 ms | 37 ms | 36 ms | 38 ms | 70712 |
| diff-fragment | 30 ms | 30 ms | 30 ms | 29 ms | 30 ms | 12805 |
| search | 55 ms | 54 ms | 54 ms | 52 ms | 80 ms | 387025 |
| tree-deep | 3 ms | 3 ms | 3 ms | 3 ms | 5 ms | 6536 |
| blob-largest | 3 ms | 1 ms | 1 ms | 1 ms | 1 ms | 5611 |
| blame-long-history | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 5385 |
| stats-overview | 82 ms | 81 ms | 81 ms | 80 ms | 91 ms | 36679 |
| stats/files | 1 ms | 0 ms | 0 ms | 0 ms | 1 ms | 24354 |
| stats/types | 0 ms | 1 ms | 1 ms | 1 ms | 1 ms | 44952 |
| stats/languages | 3.92 s | 5 ms | 5 ms | 5 ms | 6 ms | 380774 |
| stats/frequency | 698 ms | 1 ms | 1 ms | 1 ms | 1 ms | 16859 |
| stats/pulse | 164 ms | 0 ms | 0 ms | 0 ms | 1 ms | 13474 |
| stats/hotspots | 1.30 s | 1 ms | 1 ms | 0 ms | 2 ms | 111427 |
| stats/ownership | 529 ms | 1 ms | 1 ms | 0 ms | 1 ms | 19455 |
| stats/patterns | 1.03 s | 2 ms | 2 ms | 1 ms | 2 ms | 21290 |
| stats/contributors | 3 ms | 1 ms | 3 ms | 2 ms | 3 ms | 5240993 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| page_warm_ms | 181.5 | 300 | pass |
| page_cold_ms | 3919.6 | 10000 | pass |
| index_secs | 273.8 | 2400 | pass |
| idle_cpu_pct | 0.2 | 1 | pass |
| rss_mb | 2768.0 | 6000 | pass |

</details>

<!-- END GENERATED: results -->
