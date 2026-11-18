# Benchmark results

Each benchmark run (`make bench`, with `LARGE=1` to add the largest repository)
writes one `<date>-<machine>.json` here plus a folder with its charts, and
`scripts/bench/bench-report.py` regenerates this page. The method, corpus and
budgets are in [docs/benchmarks.md](../../../docs/benchmarks.md).

| Date | Machine | gitterrace | Repos | Files |
| --- | --- | --- | --- | --- |
| 2026-11-18 | Apple M1 Max, 10 cores, 64 GB RAM, macOS 26.6.2 | gitterrace v0.1.1-831e731 | control, git | [json](2026-11-18-apple-m1-max.json), [charts](2026-11-18-apple-m1-max/) |
| 2026-11-17 | Apple M1 Max, 10 cores, 64 GB RAM, macOS 26.6.2 | gitterrace v0.1.1-ddae313 | control, git, rust | [json](2026-11-17-apple-m1-max.json), [charts](2026-11-17-apple-m1-max/) |

## Newest run, 2026-11-18

Run 2026-11-18 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace v0.1.1-831e731, git version 2.54.0 (Apple Git-157).
Measured on a quiet machine.

| Repo | Metric | Value | Budget | Status | Change against previous |
| --- | --- | --- | --- | --- | --- |
| control | index build | 0.5 s (0.5 to 0.6) | 30 s | pass | unchanged |
| control | warm page | 3 ms | 300 ms | pass | 32% faster |
| control | cold page | 31 ms (29 to 37) | 1500 ms | pass | 5% slower |
| control | peak memory | 15 MB | 500 MB | pass | 25% less memory |
| control | export | 24.1 s (23.7 to 24.1) | 60 s | pass | 60% faster |
| git | index build | 11.0 s (10.7 to 11.7) | 300 s | pass | 11% slower |
| git | warm page | 12 ms | 300 ms | pass | 3% faster |
| git | cold page | 2387 ms (2385 to 2387) | 3000 ms | pass | 3% slower |
| git | peak memory | 31 MB | 1500 MB | pass | 3% more memory |
| git | export | 26.1 s (25.8 to 26.6) | 300 s | pass | 78% faster |

5 warm runs per page, 60 s idle windows.

### control

small repository, measured at the run of 2026-11-18. All 7 metrics are within
budget. Against budget: export 24.1 s (23.7 to 24.1) of 60 s, idle footprint
above a fresh server 5 MB of 48 MB, idle CPU 0.12 % of 1 %, index build 0.5 s
(0.5 to 0.6) of 30 s, cold page 31 ms (29 to 37) of 1500 ms, warm page 3 ms
(overview) of 300 ms, peak memory 15 MB of 500 MB.

Idle CPU is the larger of two windows: 0.03 % with no client and 0.12 % with one
/events client.

### git

medium repository, measured at `50fe5cc63f`. All 7 metrics are within budget.
Against budget: export 26.1 s (25.8 to 26.6) of 300 s, idle footprint above a
fresh server 5 MB of 48 MB, idle CPU 0.05 % of 1 %, index build 11.0 s (10.7 to
11.7) of 300 s, cold page 2387 ms (2385 to 2387) of 3000 ms, warm page 12 ms
(blob-largest) of 300 ms, peak memory 31 MB of 1500 MB.

Idle CPU is the larger of two windows: 0.05 % with no client and 0.02 % with one
/events client.

| Repo | Class | Measured at | Index | Peak RSS | Cache | RSS after pages | Idle footprint, total | Idle CPU | Idle CPU with /events | Export | Cold samples |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | the run of 2026-11-18 | 0.5 s | 50 MB | 1 MB | 42 MB | 10 MB | 0.03% | 0.12% | 24.1 s, 131 MB | median of 3 |
| git | medium | `50fe5cc63f` | 11.0 s | 58 MB | 45 MB | 143 MB | 10 MB | 0.05% | 0.02% | 26.1 s, 269 MB | median of 3 |

Warm page time per page kind (median), so each column compares one page:

| Page | control | git |
| --- | --- | --- |
| blame-long-history | 1 ms | 2 ms |
| blob-largest | 2 ms | 12 ms |
| commit-largest | 1 ms | 1 ms |
| diff-fragment | 3 ms | 3 ms |
| log | 3 ms | 2 ms |
| overview | 3 ms | 4 ms |
| refs | 1 ms | 1 ms |
| search | 1 ms | 1 ms |
| stats-overview | 1 ms | 1 ms |
| stats/contributors | 1 ms | 1 ms |
| stats/hotspots | 1 ms | 1 ms |
| stats/languages | 1 ms | 1 ms |
| stats/ownership | 1 ms | 1 ms |
| tree-deep | 1 ms | 1 ms |
| tree-root | 1 ms | 1 ms |

control is a working checkout, so with a tab open the server also checks it for
uncommitted changes every second. git are bare clones with no working tree, so
they have nothing to check and idle lower.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-18-apple-m1-max/page_warm_ms-dark.svg">
<img alt="Warm page, slowest median per repository against its budget" src="2026-11-18-apple-m1-max/page_warm_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-18-apple-m1-max/page_cold_ms-dark.svg">
<img alt="Cold page, slowest per repository against its budget" src="2026-11-18-apple-m1-max/page_cold_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-18-apple-m1-max/index_secs-dark.svg">
<img alt="Index build per repository against its budget" src="2026-11-18-apple-m1-max/index_secs.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-18-apple-m1-max/idle_cpu_pct-dark.svg">
<img alt="Idle CPU, worse of plain and /events client per repository against its budget" src="2026-11-18-apple-m1-max/idle_cpu_pct.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-18-apple-m1-max/rss_mb-dark.svg">
<img alt="Peak physical footprint (index peak or after pages) per repository against its budget" src="2026-11-18-apple-m1-max/rss_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-18-apple-m1-max/footprint_idle_mb-dark.svg">
<img alt="Idle footprint above a fresh server per repository against its budget" src="2026-11-18-apple-m1-max/footprint_idle_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-18-apple-m1-max/export_secs-dark.svg">
<img alt="Export per repository against its budget" src="2026-11-18-apple-m1-max/export_secs.svg">
</picture>

### Compared with 2026-11-17

The previous run, on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). Measured on a quiet machine. Its charts stay beside its result
file. On the charts above, a hollow marker shows its value.

| Repo | Metric | 2026-11-17 | 2026-11-18 | Change |
| --- | --- | --- | --- | --- |
| control | export | 60.2 s | 24.1 s | 60% faster |
| control | idle footprint above a fresh server | 6 MB | 5 MB | 16% less memory |
| control | idle CPU | 0.08 % | 0.12 % | 50% more CPU |
| control | index build | 0.5 s | 0.5 s | unchanged |
| control | cold page | 29 ms | 31 ms | 5% slower |
| control | warm page | 5 ms | 3 ms | 32% faster |
| control | peak memory | 20 MB | 15 MB | 25% less memory |
| git | export | 119.2 s | 26.1 s | 78% faster |
| git | idle footprint above a fresh server | 5 MB | 5 MB | 9% more memory |
| git | idle CPU | 0.08 % | 0.05 % | 38% less CPU |
| git | index build | 9.9 s | 11.0 s | 11% slower |
| git | cold page | 2311 ms | 2387 ms | 3% slower |
| git | warm page | 13 ms | 12 ms | 3% faster |
| git | peak memory | 30 MB | 31 MB | 3% more memory |

<details><summary>control: every page</summary>

Probes: `crates/gitterrace-web/tests/golden/revisions.txt` (largest text file),
commit the run of 2026-11-18.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes | Previous warm |
| --- | --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 4 ms | 1 ms | 1 ms | 1 ms | 1 ms | 342260 | 1 ms |
| blob-largest | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 180849 | 2 ms |
| commit-largest | 13 ms | 1 ms | 1 ms | 1 ms | 1 ms | 64229 | 1 ms |
| diff-fragment | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 13752 | 5 ms |
| log | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 53380 | 3 ms |
| overview | 16 ms | 3 ms | 3 ms | 3 ms | 3 ms | 38949 | 3 ms |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 19847 | 1 ms |
| search | 31 ms | 1 ms | 1 ms | 1 ms | 1 ms | 349587 | 1 ms |
| stats-overview | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 132041 | 1 ms |
| stats/contributors | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 21557 | 1 ms |
| stats/hotspots | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 258957 | 1 ms |
| stats/languages | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 440859 | 1 ms |
| stats/ownership | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 28334 | 1 ms |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 19487 | 1 ms |
| tree-root | 3 ms | 1 ms | 1 ms | 1 ms | 1 ms | 35546 | 1 ms |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 24.1 | 60 | pass |
| footprint_idle_mb | 5.2 | 48 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 0.5 | 30 | pass |
| page_cold_ms | 30.7 | 1500 | pass |
| page_warm_ms | 3.2 | 300 | pass |
| rss_mb | 15.0 | 500 | pass |

</details>

<details><summary>git: every page</summary>

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes | Previous warm |
| --- | --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 2.39 s | 1 ms | 2 ms | 2 ms | 2 ms | 709528 | 2 ms |
| blob-largest | 13 ms | 12 ms | 12 ms | 12 ms | 12 ms | 1760757 | 13 ms |
| commit-largest | 7 ms | 1 ms | 1 ms | 1 ms | 1 ms | 22824 | 1 ms |
| diff-fragment | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 4232 | 3 ms |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 51374 | 2 ms |
| overview | 98 ms | 4 ms | 4 ms | 4 ms | 5 ms | 148684 | 4 ms |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 17994 | 1 ms |
| search | 58 ms | 1 ms | 1 ms | 1 ms | 1 ms | 247609 | 1 ms |
| stats-overview | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 134835 | 1 ms |
| stats/contributors | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 95739 | 1 ms |
| stats/hotspots | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 222787 | 1 ms |
| stats/languages | 1 ms | 1 ms | 1 ms | 1 ms | 2 ms | 409501 | 1 ms |
| stats/ownership | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 36007 | 1 ms |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 18649 | 1 ms |
| tree-root | 5 ms | 1 ms | 1 ms | 1 ms | 1 ms | 145210 | 1 ms |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 26.1 | 300 | pass |
| footprint_idle_mb | 5.0 | 48 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 11.0 | 300 | pass |
| page_cold_ms | 2386.8 | 3000 | pass |
| page_warm_ms | 12.3 | 300 | pass |
| rss_mb | 31.0 | 1500 | pass |

</details>
