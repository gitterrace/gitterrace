# Benchmark results

Each benchmark run (`make bench`, with `LARGE=1` to add the largest repository)
writes one `<date>-<machine>.json` here plus a folder with its charts, and
`scripts/bench/bench-report.py` regenerates this page. The method, corpus and
budgets are in [docs/benchmarks.md](../../../docs/benchmarks.md).

| Date | Machine | gitterrace | Repos | Files |
| --- | --- | --- | --- | --- |
| 2026-11-17 | Apple M1 Max, 10 cores, 64 GB RAM, macOS 26.6.2 | gitterrace v0.1.1-ddae313 | control, git, rust | [json](2026-11-17-apple-m1-max.json), [charts](2026-11-17-apple-m1-max/) |
| 2026-11-13 | Apple M1 Max, 10 cores, 64 GB RAM, macOS 26.6.2 | gitterrace v0.1.0-46ca3fd | control, git, linux | [json](2026-11-13-apple-m1-max.json), [charts](2026-11-13-apple-m1-max/) |

## Newest run, 2026-11-17

Run 2026-11-17 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace v0.1.1-ddae313, git version 2.54.0 (Apple Git-157).
Measured on a quiet machine.

- **linux:** not measured on a quiet machine (other processes used 5 or more
  cores during export), so its numbers are withheld.

| Repo | Metric | Value | Budget | Status | Change against previous |
| --- | --- | --- | --- | --- | --- |
| control | index build | 0.5 s (0.5 to 0.6) | 30 s | pass | 10% faster |
| control | warm page | 5 ms | 300 ms | pass | 97% faster |
| control | cold page | 29 ms (29 to 31) | 1500 ms | pass | 81% faster |
| control | peak memory | 20 MB | 500 MB | pass | 82% less memory |
| control | export | 60.2 s (59.8 to 60.4) | 60 s | over | 57% slower |
| git | index build | 9.9 s (9.8 to 10.5) | 300 s | pass | 59% faster |
| git | warm page | 13 ms | 300 ms | pass | 86% faster |
| git | cold page | 2311 ms (2310 to 2332) | 3000 ms | pass | 3% faster |
| git | peak memory | 30 MB | 1500 MB | pass | 92% less memory |
| git | export | 119.2 s (119.0 to 122.3) | 300 s | pass | 94% slower |
| rust | index build | 62.8 s (62.5 to 64.5) | 2400 s | pass | new in this run |
| rust | warm page | 28 ms | 300 ms | pass | new in this run |
| rust | cold page | 854 ms (829 to 894) | 10000 ms | pass | new in this run |
| rust | peak memory | 122 MB | 6000 MB | pass | new in this run |
| linux | index build | withheld (load) | - | withheld | - |
| linux | warm page | withheld (load) | - | withheld | - |
| linux | cold page | withheld (load) | - | withheld | - |
| linux | peak memory | withheld (load) | - | withheld | - |
| linux | export | withheld (load) | - | withheld | - |

5 warm runs per page, 60 s idle windows.

### control

small repository, measured at the run of 2026-11-17. 6 of 7 metrics are within
budget. Against budget: export 60.2 s (59.8 to 60.4) of 60 s, idle footprint
above a fresh server 6 MB of 48 MB, idle CPU 0.08 % of 1 %, index build 0.5 s
(0.5 to 0.6) of 30 s, cold page 29 ms (29 to 31) of 1500 ms, warm page 5 ms
(diff-fragment) of 300 ms, peak memory 20 MB of 500 MB.

Idle CPU is the larger of two windows: 0.07 % with no client and 0.08 % with one
/events client.

- **Over budget:** export is 60.2 s against a limit of 60 s (1.0x).

### git

medium repository, measured at `50fe5cc63f`. All 7 metrics are within budget.
Against budget: export 119.2 s (119.0 to 122.3) of 300 s, idle footprint above a
fresh server 5 MB of 48 MB, idle CPU 0.08 % of 1 %, index build 9.9 s (9.8 to
10.5) of 300 s, cold page 2311 ms (2310 to 2332) of 3000 ms, warm page 13 ms
(blob-largest) of 300 ms, peak memory 30 MB of 1500 MB.

Idle CPU is the larger of two windows: 0.08 % with no client and 0.02 % with one
/events client.

### rust

large repository, measured at `f90a2d27c2`. All 6 metrics are within budget.
Against budget: idle footprint above a fresh server 9 MB of 48 MB, idle CPU 0.12
% of 1 %, index build 62.8 s (62.5 to 64.5) of 2400 s, cold page 854 ms (829 to
894) of 10000 ms, warm page 28 ms (diff-fragment) of 300 ms, peak memory 122 MB
of 6000 MB.

Idle CPU is the larger of two windows: 0.12 % with no client and 0.02 % with one
/events client.

| Repo | Class | Measured at | Index | Peak RSS | Cache | RSS after pages | Idle footprint, total | Idle CPU | Idle CPU with /events | Export | Cold samples |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| control | small | the run of 2026-11-17 | 0.5 s | 37 MB | 1 MB | 42 MB | 11 MB | 0.07% | 0.08% | 60.2 s, 129 MB | median of 3 |
| git | medium | `50fe5cc63f` | 9.9 s | 88 MB | 45 MB | 143 MB | 9 MB | 0.08% | 0.02% | 119.2 s, 269 MB | median of 3 |
| rust | large | `f90a2d27c2` | 62.8 s | 245 MB | 295 MB | 272 MB | 14 MB | 0.12% | 0.02% | 1859.2 s, 1 run, 2299 MB | median of 3 |
| linux | huge | `cbbddb6b08` | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | withheld (load) | 1 sample |

Warm page time per page kind (median), so each column compares one page:

| Page | control | git | rust |
| --- | --- | --- | --- |
| blame-long-history | 1 ms | 2 ms | 1 ms |
| blob-largest | 2 ms | 13 ms | 16 ms |
| commit-largest | 1 ms | 1 ms | 1 ms |
| diff-fragment | 5 ms | 3 ms | 28 ms |
| log | 3 ms | 2 ms | 2 ms |
| overview | 3 ms | 4 ms | 3 ms |
| refs | 1 ms | 1 ms | 1 ms |
| search | 1 ms | 1 ms | 1 ms |
| stats-overview | 1 ms | 1 ms | 1 ms |
| stats/contributors | 1 ms | 1 ms | 1 ms |
| stats/hotspots | 1 ms | 1 ms | 1 ms |
| stats/languages | 1 ms | 1 ms | 1 ms |
| stats/ownership | 1 ms | 1 ms | 1 ms |
| tree-deep | 1 ms | 1 ms | 1 ms |
| tree-root | 1 ms | 1 ms | 1 ms |

control is a working checkout, so with a tab open the server also checks it for
uncommitted changes every second. git, rust, linux are bare clones with no
working tree, so they have nothing to check and idle lower.

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-17-apple-m1-max/page_warm_ms-dark.svg">
<img alt="Warm page, slowest median per repository against its budget" src="2026-11-17-apple-m1-max/page_warm_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-17-apple-m1-max/page_cold_ms-dark.svg">
<img alt="Cold page, slowest per repository against its budget" src="2026-11-17-apple-m1-max/page_cold_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-17-apple-m1-max/index_secs-dark.svg">
<img alt="Index build per repository against its budget" src="2026-11-17-apple-m1-max/index_secs.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-17-apple-m1-max/idle_cpu_pct-dark.svg">
<img alt="Idle CPU, worse of plain and /events client per repository against its budget" src="2026-11-17-apple-m1-max/idle_cpu_pct.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-17-apple-m1-max/rss_mb-dark.svg">
<img alt="Peak physical footprint (index peak or after pages) per repository against its budget" src="2026-11-17-apple-m1-max/rss_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-17-apple-m1-max/footprint_idle_mb-dark.svg">
<img alt="Idle footprint above a fresh server per repository against its budget" src="2026-11-17-apple-m1-max/footprint_idle_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="2026-11-17-apple-m1-max/export_secs-dark.svg">
<img alt="Export per repository against its budget" src="2026-11-17-apple-m1-max/export_secs.svg">
</picture>

### Compared with 2026-11-13

The previous run, on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). Measured on a quiet machine. Its charts stay beside its result
file. On the charts above, a hollow marker shows its value.

| Repo | Metric | 2026-11-13 | 2026-11-17 | Change |
| --- | --- | --- | --- | --- |
| control | export | 38.4 s | 60.2 s | 57% slower |
| control | idle footprint above a fresh server | 13 MB | 6 MB | 53% less memory |
| control | idle CPU | 0.08 % | 0.08 % | unchanged |
| control | index build | 0.6 s | 0.5 s | 10% faster |
| control | cold page | 156 ms | 29 ms | 81% faster |
| control | warm page | 145 ms | 5 ms | 97% faster |
| control | peak memory | 114 MB | 20 MB | 82% less memory |
| git | export | 61.3 s | 119.2 s | 94% slower |
| git | idle footprint above a fresh server | 8 MB | 5 MB | 45% less memory |
| git | idle CPU | 0.05 % | 0.08 % | 60% more CPU |
| git | index build | 24.2 s | 9.9 s | 59% faster |
| git | cold page | 2378 ms | 2311 ms | 3% faster |
| git | warm page | 87 ms | 13 ms | 86% faster |
| git | peak memory | 366 MB | 30 MB | 92% less memory |
| rust | idle footprint above a fresh server | not run | 9 MB | new in this run |
| rust | idle CPU | not run | 0.12 % | new in this run |
| rust | index build | not run | 62.8 s | new in this run |
| rust | cold page | not run | 854 ms | new in this run |
| rust | warm page | not run | 28 ms | new in this run |
| rust | peak memory | not run | 122 MB | new in this run |

<details><summary>control: every page</summary>

Probes: `crates/gitterrace-web/tests/golden/revisions.txt` (largest text file),
commit the run of 2026-11-17.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes | Previous warm |
| --- | --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 4 ms | 1 ms | 1 ms | 1 ms | 1 ms | 342253 | 1 ms |
| blob-largest | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 180842 | 2 ms |
| commit-largest | 14 ms | 1 ms | 1 ms | 1 ms | 1 ms | 79022 | 145 ms |
| diff-fragment | 5 ms | 5 ms | 5 ms | 5 ms | 5 ms | 33321 | 4 ms |
| log | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 53245 | 1 ms |
| overview | 8 ms | 3 ms | 3 ms | 3 ms | 4 ms | 39048 | 2 ms |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 19855 | 1 ms |
| search | 29 ms | 1 ms | 1 ms | 1 ms | 1 ms | 349580 | 1 ms |
| stats-overview | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 130130 | 0 ms |
| stats/contributors | 1 ms | 1 ms | 1 ms | 0 ms | 1 ms | 21539 | 0 ms |
| stats/hotspots | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 258941 | 1 ms |
| stats/languages | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 440922 | 1 ms |
| stats/ownership | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 28327 | 0 ms |
| tree-deep | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 19480 | 1 ms |
| tree-root | 3 ms | 1 ms | 1 ms | 1 ms | 1 ms | 35539 | 0 ms |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 60.2 | 60 | over |
| footprint_idle_mb | 6.2 | 48 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 0.5 | 30 | pass |
| page_cold_ms | 29.3 | 1500 | pass |
| page_warm_ms | 4.8 | 300 | pass |
| rss_mb | 20.0 | 500 | pass |

</details>

<details><summary>git: every page</summary>

Probes: `po/bg.po` (largest text file), commit `50fe5cc63f`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes | Previous warm |
| --- | --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 2.31 s | 1 ms | 2 ms | 1 ms | 2 ms | 709528 | 87 ms |
| blob-largest | 13 ms | 12 ms | 13 ms | 12 ms | 13 ms | 1760757 | 13 ms |
| commit-largest | 6 ms | 1 ms | 1 ms | 1 ms | 1 ms | 22824 | 1 ms |
| diff-fragment | 3 ms | 3 ms | 3 ms | 3 ms | 3 ms | 4232 | 3 ms |
| log | 2 ms | 2 ms | 2 ms | 2 ms | 2 ms | 51374 | 2 ms |
| overview | 94 ms | 4 ms | 4 ms | 4 ms | 5 ms | 148618 | 4 ms |
| refs | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 17994 | 1 ms |
| search | 49 ms | 1 ms | 1 ms | 1 ms | 1 ms | 247609 | 1 ms |
| stats-overview | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 134835 | 1 ms |
| stats/contributors | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 95739 | 1 ms |
| stats/hotspots | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 222787 | 1 ms |
| stats/languages | 2 ms | 1 ms | 1 ms | 1 ms | 2 ms | 409501 | 1 ms |
| stats/ownership | 1 ms | 1 ms | 1 ms | 1 ms | 1 ms | 36007 | 0 ms |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 18649 | 0 ms |
| tree-root | 4 ms | 1 ms | 1 ms | 1 ms | 1 ms | 145210 | 1 ms |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| export_secs | 119.2 | 300 | pass |
| footprint_idle_mb | 4.5 | 48 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 9.9 | 300 | pass |
| page_cold_ms | 2311.4 | 3000 | pass |
| page_warm_ms | 12.6 | 300 | pass |
| rss_mb | 30.0 | 1500 | pass |

</details>

<details><summary>rust: every page</summary>

Probes: `library/stdarch/crates/core_arch/src/aarch64/neon/generated.rs`
(largest text file), commit `f90a2d27c2`.

| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |
| --- | --- | --- | --- | --- | --- | --- |
| blame-long-history | 615 ms | 1 ms | 1 ms | 1 ms | 2 ms | 615530 |
| blob-largest | 16 ms | 14 ms | 16 ms | 15 ms | 16 ms | 2394284 |
| commit-largest | 74 ms | 1 ms | 1 ms | 1 ms | 2 ms | 89967 |
| diff-fragment | 29 ms | 28 ms | 28 ms | 27 ms | 28 ms | 14446 |
| log | 3 ms | 2 ms | 2 ms | 2 ms | 2 ms | 51100 |
| overview | 854 ms | 3 ms | 3 ms | 3 ms | 4 ms | 36235 |
| refs | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 18074 |
| search | 91 ms | 1 ms | 1 ms | 1 ms | 1 ms | 303511 |
| stats-overview | 477 ms | 1 ms | 1 ms | 1 ms | 2 ms | 126700 |
| stats/contributors | 55 ms | 1 ms | 1 ms | 1 ms | 1 ms | 91460 |
| stats/hotspots | 332 ms | 1 ms | 1 ms | 1 ms | 1 ms | 277661 |
| stats/languages | 531 ms | 1 ms | 1 ms | 1 ms | 2 ms | 439756 |
| stats/ownership | 83 ms | 1 ms | 1 ms | 1 ms | 1 ms | 36041 |
| tree-deep | 2 ms | 1 ms | 1 ms | 1 ms | 1 ms | 19637 |
| tree-root | 3 ms | 1 ms | 1 ms | 1 ms | 1 ms | 32564 |

| Budget | Value | Limit | Status |
| --- | --- | --- | --- |
| footprint_idle_mb | 9.2 | 48 | pass |
| idle_cpu_pct | 0.1 | 1 | pass |
| index_secs | 62.8 | 2400 | pass |
| page_cold_ms | 854.2 | 10000 | pass |
| page_warm_ms | 27.7 | 300 | pass |
| rss_mb | 122.0 | 6000 | pass |

</details>
