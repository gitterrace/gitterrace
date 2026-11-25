#!/usr/bin/env python3
"""The budget and gate tables of docs/benchmarks.md for scripts/bench/bench-report.py.
Imported, not run.
"""

from __future__ import annotations

import importlib

bc = importlib.import_module("bench-chart-lib")
bl = importlib.import_module("bench-lib")
bq = importlib.import_module("bench-quiet-lib")


BUDGET_COLUMNS = [
    ("page_warm_ms", "Warm ms"),
    ("page_cold_ms", "Cold ms"),
    ("index_secs", "Index s"),
    ("ready_secs", "Ready s"),
    ("idle_cpu_pct", "Idle CPU %"),
    ("rss_mb", "Peak MB"),
    ("footprint_idle_mb", "Idle MB"),
    ("export_secs", "Export s"),
]


def budgets_md() -> str:
    """The budget table per size class from corpus.toml, and the run defaults the harness uses."""
    budgets = bl.parse_toml(bc.CORPUS.read_text())["budgets"]
    head = "| Class | " + " | ".join(t for _, t in BUDGET_COLUMNS) + " |"
    rule = "| --- |" + " --- |" * len(BUDGET_COLUMNS)
    rows = [
        f"| {cls} | " + " | ".join(str(b.get(k, "-")) for k, _ in BUDGET_COLUMNS) + " |"
        for cls, b in budgets.items()
    ]
    return (
        "\n".join([head, rule, *rows])
        + f"\n\nA published run takes the median of {bq.FULL_RUNS} warm runs and idles over a "
        f"{bq.FULL_IDLE_SECS} s window. The newest {bc.KEEP} runs are kept.\n"
    )


def gates_md() -> str:
    """The cold insights page targets, the gates `make speed-check` fails on."""
    sc = importlib.import_module("speed-check")
    rows = ["| Repository | Page | Target (ms) |", "| --- | --- | --- |"]
    for repo, pages in sc.GATES.items():
        rows += [f"| {repo} | {page} | {ms:g} |" for page, ms in pages.items()]
    return "\n".join(rows) + "\n"
