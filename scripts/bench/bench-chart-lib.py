#!/usr/bin/env python3
"""Constants, number formatting, run normalizing and chart rendering for
scripts/bench/bench-report.py. Imported, not run.
"""

from __future__ import annotations

import importlib
import json
import re
import sys
from html import escape
from pathlib import Path
from typing import Any

Json = dict[str, Any]

bl = importlib.import_module("bench-lib")

ROOT = Path(__file__).resolve().parents[2]
DOC = ROOT / "docs/benchmarks.md"
DOCS = ROOT / "docs"
CORPUS = ROOT / "tests/bench/corpus.toml"
RESULTS = ROOT / "tests/bench/results"
BEGIN, END = "<!-- BEGIN GENERATED: results -->", "<!-- END GENERATED: results -->"
# Result files are <date>-<machine label>.json. The label comes from the CPU, never the host.
NAME = re.compile(r"\d{4}-\d{2}-\d{2}-[a-z0-9-]+\.json")
# The newest run and its baseline. Older runs live in git history.
KEEP = 2
# Floats are rounded to this many decimals, so diffs between runs stay readable.
DECIMALS = 2

# metric key -> (title, unit, value format)
METRICS = {
    "page_warm_ms": ("Warm page, slowest median", "ms", "{:.0f}"),
    "page_cold_ms": ("Cold page, slowest", "ms", "{:.0f}"),
    "index_secs": ("Index build", "s", "{:.1f}"),
    "ready_secs": ("Add to ready", "s", "{:.1f}"),
    "idle_cpu_pct": ("Idle CPU, worse of plain and /events client", "%", "{:.2f}"),
    "rss_mb": ("Peak physical footprint (index peak or after pages)", "MB", "{:.0f}"),
    "footprint_idle_mb": ("Idle footprint above a fresh server", "MB", "{:.0f}"),
    "export_secs": ("Export", "s", "{:.1f}"),
}
# metric key -> result fields it reads, dotted. The gate takes the max of the listed fields,
# and the bench-report self-test fails when one of them can go over budget unnoticed.
SOURCES = {
    "idle_cpu_pct": ("idle.cpu_pct", "idle.cpu_pct_events_client"),
    # Physical footprint, not RSS: clean mapped pack files are left out, the system can take
    # them back. RSS is still recorded beside it (index.peak_rss_mb).
    "rss_mb": ("index.peak_footprint_mb", "memory.footprint_after_pages_mb"),
}
SHORT = {
    "page_warm_ms": "warm page",
    "page_cold_ms": "cold page",
    "index_secs": "index build",
    "ready_secs": "add to ready",
    "idle_cpu_pct": "idle CPU",
    "rss_mb": "peak memory",
    "footprint_idle_mb": "idle footprint above a fresh server",
    "export_secs": "export",
}
# Schwarzwald-style calm tones per scheme: (ok bar, over bar, text).
PALETTE = {
    "light": ("#5f8a6e", "#b5654a", "#2b2b2b"),
    "dark": ("#8fb89c", "#d98b72", "#e6e6e6"),
}


def ms(x: float) -> str:
    return f"{x / 1000:.2f} s" if x >= 1000 else f"{x:.0f} ms"


def num(key: str, v: float) -> str:
    return METRICS[key][2].format(v)


def fmt(key: str, v: float) -> str:
    return f"{num(key, v)} {METRICS[key][1]}"


# Corpus -------------------------------------------------------------------------------------


def corpus_repos() -> list[dict[str, Any]]:
    """The [[repo]] tables of corpus.toml, in corpus order."""
    return bl.parse_toml(CORPUS.read_text())["repo"]


def corpus() -> list[tuple[str, str, str]]:
    """(name, source page, why) per repo, from corpus.toml so the doc list can't drift."""
    repos = corpus_repos()
    for r in repos:
        if not r.get("url") or not r.get("why"):
            sys.exit(f"tests/bench/corpus.toml: repo {r.get('name')} needs a url and a why")
        if r.get("local") and r.get("sha"):
            sys.exit(
                f"tests/bench/corpus.toml: {r['name']} is local, measured at HEAD: drop its sha"
            )
    return [(r["name"], re.sub(r"\.git$", "", r["url"]), r["why"]) for r in repos]


def corpus_list() -> str:
    return "\n".join(f"- [**{n}**]({u}): {w}" for n, u, w in corpus()) + "\n"


# Result files -------------------------------------------------------------------------------


def rounded(v: Any) -> Any:
    if isinstance(v, float):
        return round(v, DECIMALS)
    if isinstance(v, dict):
        return {k: rounded(x) for k, x in v.items()}
    if isinstance(v, list):
        return [rounded(x) for x in v]
    return v


def os_label(os_name: str) -> str:
    """Old runs recorded the Darwin kernel. Darwin N is macOS N + 1 from macOS 26 on."""
    m = re.fullmatch(r"Darwin (\d+)\.\d+\.\d+", os_name)
    return f"macOS {int(m.group(1)) + 1}" if m and int(m.group(1)) >= 25 else os_name


def label_of(cpu: str, arch: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", cpu.lower()).strip("-") or arch


def normalize(run: Json) -> Json:
    """The published form of a run: no hostname, a date, repos in corpus order, rounded floats.

    Applies to old runs too, so a file from before this format converts without re-measuring."""
    run = json.loads(json.dumps(run))
    m = run["machine"]
    m.pop("host", None)
    m["os"] = os_label(m["os"])
    m["label"] = m.get("label") or label_of(m["cpu"], m.get("arch", ""))
    run["when"] = run["when"][:10]
    order = [r["name"] for r in corpus_repos()]
    repos = run["repos"]
    run["repos"] = {
        n: repos[n] for n in sorted(repos, key=lambda n: (order.index(n) if n in order else 99, n))
    }
    return rounded(run)


def dumps(run: Json) -> str:
    """Sorted keys, corpus order for the repos map, one line per metric (objects four levels
    down stay on one line), so a run is a few hundred lines and still diffs per metric."""

    def emit(v: Any, depth: int, keep_order: bool = False) -> str:
        if depth >= 4 or not isinstance(v, dict) or not v:
            return json.dumps(v, sort_keys=True)
        pad = " " * (depth + 1)
        items = v.items() if keep_order else sorted(v.items())
        body = ",\n".join(
            f"{pad}{json.dumps(k)}: {emit(x, depth + 1, depth == 0 and k == 'repos')}"
            for k, x in items
        )
        return "{\n" + body + "\n" + " " * depth + "}"

    return emit(run, 0) + "\n"


def run_name(run: Json) -> str:
    return f"{run['when'][:10]}-{run['machine']['label']}"


def runs() -> list[Path]:
    return sorted(RESULTS.glob("*.json"))


def load(path: Path) -> Json:
    return json.loads(path.read_text())


# Charts -------------------------------------------------------------------------------------


# Words for a change, so the table never relies on colour or arrows: (lower, higher).
DIRECTION = {
    "page_warm_ms": ("faster", "slower"),
    "page_cold_ms": ("faster", "slower"),
    "index_secs": ("faster", "slower"),
    "ready_secs": ("faster", "slower"),
    "export_secs": ("faster", "slower"),
    "idle_cpu_pct": ("less CPU", "more CPU"),
    "rss_mb": ("less memory", "more memory"),
    "footprint_idle_mb": ("less memory", "more memory"),
}


def chart(key: str, repos: Json, scheme: str, prev: Json | None = None) -> str | None:
    """Bars against the budget line. With prev, a hollow marker shows the previous run's value."""
    title, unit, _ = METRICS[key]
    ok, over, ink = PALETTE[scheme]
    rows = [(n, r["budget"][key]) for n, r in repos.items() if key in r["budget"]]
    if not rows:
        return None
    before = {
        n: r["budget"][key]["value"] for n, r in (prev or {}).items() if key in r.get("budget", {})
    }
    top = max(max(b["value"], b["limit"], before.get(n, 0)) for n, b in rows) * 1.08 or 1
    lx, bx, bw, rh, pad = 8, 84, 380, 34, 30
    w, h = 640, pad + rh * len(rows) + 22
    font = f'fill="{ink}" font-family="sans-serif"'
    s = [
        f'<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 {w} {h}" width="{w}" '
        f'height="{h}" aria-label="{escape(title)} per repository against its budget">',
        f"<title>{escape(title)} ({unit}) against budget</title>",
        f'<text x="{lx}" y="16" {font} font-size="13" font-weight="600">'
        f"{escape(title)} ({unit})</text>",
    ]
    for i, (n, b) in enumerate(rows):
        y = pad + i * rh
        passed = b["status"] == "pass"
        bar = max(bw * b["value"] / top, 1.5)
        lim = bw * b["limit"] / top
        s += [
            f'<text x="{lx}" y="{y + 17}" {font} font-size="12">{escape(n)}</text>',
            f'<rect x="{bx}" y="{y + 4}" width="{bar:.1f}" height="18" rx="2" '
            f'fill="{ok if passed else over}"/>',
            f'<line x1="{bx + lim:.1f}" y1="{y}" x2="{bx + lim:.1f}" y2="{y + 26}" '
            f'stroke="{ink}" stroke-width="1.5" stroke-dasharray="3 2"/>',
            f'<text x="{bx + bw + 14}" y="{y + 17}" {font} font-size="12">'
            f"{num(key, b['value'])} of {b['limit']}{'' if passed else ' (over)'}</text>",
        ]
        if "min" in b and b["max"] > b["min"]:
            x1, x2 = bx + bw * b["min"] / top, bx + bw * b["max"] / top
            s.append(
                f'<path d="M{x1:.1f} {y + 13}H{x2:.1f}M{x1:.1f} {y + 9}V{y + 17}'
                f'M{x2:.1f} {y + 9}V{y + 17}" stroke="{ink}" stroke-width="1" fill="none">'
                f"<title>spread {num(key, b['min'])} to {num(key, b['max'])}</title></path>"
            )
        if n in before:
            x = bx + bw * before[n] / top
            s.append(
                f'<rect x="{x - 2:.1f}" y="{y + 1}" width="4" height="24" fill="none" '
                f'stroke="{ink}" stroke-width="1.5"><title>previous run: '
                f"{num(key, before[n])}</title></rect>"
            )
    s.append(
        f'<text x="{bx}" y="{h - 6}" {font} font-size="11" opacity="0.7">'
        "dashed line: budget"
        + ("    whisker: min to max" if any("min" in b for _, b in rows) else "")
        + ("    hollow marker: previous run" if before else "")
        + "</text></svg>"
    )
    return "\n".join(s) + "\n"


def charts(run: Json, prev: Json | None = None) -> dict[str, str]:
    """File name -> SVG for one run, light and dark per metric, marking prev's values."""
    out: dict[str, str] = {}
    before = prev["repos"] if prev else None
    for key in METRICS:
        for scheme, suffix in (("light", ""), ("dark", "-dark")):
            svg = chart(key, run["repos"], scheme, before)
            if svg:
                out[f"{key}{suffix}.svg"] = svg
    return out


def picture(run: Json, key: str, d: str | None = None) -> list[str]:
    """A light and dark chart. d is the chart folder relative to the page that shows it."""
    d = f"../tests/bench/results/{run_name(run)}" if d is None else d
    title = METRICS[key][0]
    return [
        "<picture>",
        f'<source media="(prefers-color-scheme: dark)" srcset="{d}/{key}-dark.svg">',
        f'<img alt="{escape(title)} per repository against its budget" src="{d}/{key}.svg">',
        "</picture>",
        "",
    ]


def by_page(run: Json) -> list[str]:
    """Warm median per page kind across repositories: the same page compared with itself. The
    headline is each repo's slowest page, which is a different page from repo to repo."""
    repos = run["repos"]
    kinds = list(dict.fromkeys(k for r in repos.values() for k in r["pages"]))
    if not kinds:
        return []
    L = [
        "Warm page time per page kind (median), so each column compares one page:",
        "",
        "| Page | " + " | ".join(repos) + " |",
        "| --- |" + " --- |" * len(repos),
    ]
    for k in kinds:
        cells = [
            ms(r["pages"][k]["warm_total_ms"]["median"]) if k in r["pages"] else "-"
            for r in repos.values()
        ]
        L.append(f"| {k} | " + " | ".join(cells) + " |")
    return L + [""]


def slowest_page_test(br: Any) -> None:
    """The page budget names the page that set it, and the per-page table puts one page kind
    of every repo on one row, so a reader compares a page with the same page."""
    pg = lambda w: {"warm_total_ms": {"median": w}, "cold": {"total_ms": 1}}  # noqa: E731
    res: Json = {
        "class": "c",
        "pages": {"blame": pg(90), "diff": pg(10)},
        "index": {"secs": 1, "peak_rss_mb": 1, "peak_footprint_mb": 1},
        "idle": {"cpu_pct": 0.1, "cpu_pct_events_client": 0.1},
        "memory": {"rss_after_pages_mb": 1, "footprint_after_pages_mb": 1},
    }
    got = br.judge(res, {"c": {"page_warm_ms": 300}})["page_warm_ms"].get("page")
    if got != "blame":
        sys.exit(f"bench-report self-test failed: the slowest page reads {got}")
    other = {**res, "pages": {"blame": pg(5), "diff": pg(70)}}
    table = by_page({"repos": {"a": res, "b": other}})
    if "| blame | 90 ms | 5 ms |" not in table or "| diff | 10 ms | 70 ms |" not in table:
        sys.exit(f"bench-report self-test failed: per-page rows {table}")
