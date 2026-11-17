# Benchmark run 2026-11-17-apple-m1-max

Run 2026-11-17 on Apple M1 Max, 10 cores, 64 GB RAM, 4.0 TB SSD disk, macOS
26.6.2 (arm64). gitterrace v0.1.1-ddae313, git version 2.54.0 (Apple Git-157).
Measured on a quiet machine.

- **linux:** not measured on a quiet machine (other processes used 5 or more
  cores during export), so its numbers are withheld.

Numbers: [`../2026-11-17-apple-m1-max.json`](../2026-11-17-apple-m1-max.json).
Method: [docs/benchmarks.md](../../../../docs/benchmarks.md).

<picture>
<source media="(prefers-color-scheme: dark)" srcset="./page_warm_ms-dark.svg">
<img alt="Warm page, slowest median per repository against its budget" src="./page_warm_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="./page_cold_ms-dark.svg">
<img alt="Cold page, slowest per repository against its budget" src="./page_cold_ms.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="./index_secs-dark.svg">
<img alt="Index build per repository against its budget" src="./index_secs.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="./idle_cpu_pct-dark.svg">
<img alt="Idle CPU, worse of plain and /events client per repository against its budget" src="./idle_cpu_pct.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="./rss_mb-dark.svg">
<img alt="Peak physical footprint (index peak or after pages) per repository against its budget" src="./rss_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="./footprint_idle_mb-dark.svg">
<img alt="Idle footprint above a fresh server per repository against its budget" src="./footprint_idle_mb.svg">
</picture>

<picture>
<source media="(prefers-color-scheme: dark)" srcset="./export_secs-dark.svg">
<img alt="Export per repository against its budget" src="./export_secs.svg">
</picture>
