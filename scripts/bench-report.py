#!/usr/bin/env python3
"""Turn the newest bench/results/*.json into the generated block of docs/benchmarks.md.

The block is a written summary per repo (each metric against its budget, over-budget rows
called out), a compact table, one inline SVG bar chart per metric, and the per-page tables
folded away. Stdlib only, no PNG, no JS, no fonts: the SVG text uses currentColor and the bars
switch palette with prefers-color-scheme.

  scripts/bench-report.py            rewrite the block in docs/benchmarks.md  (make bench-report)
  scripts/bench-report.py --check    fail when the block is stale; writes nothing
  scripts/bench-report.py --print    print the block (what `cargo xtask` splices in)
"""

import json
import re
import sys
import textwrap
from html import escape
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOC = ROOT / "docs/benchmarks.md"
BEGIN, END = "<!-- BEGIN GENERATED: results -->", "<!-- END GENERATED: results -->"

# metric key -> (title, unit, value format)
METRICS = {
    "page_warm_ms": ("Warm page, slowest median", "ms", "{:.0f}"),
    "page_cold_ms": ("Cold page, slowest", "ms", "{:.0f}"),
    "index_secs": ("Index build", "s", "{:.1f}"),
    "idle_cpu_pct": ("Idle CPU with /events client", "%", "{:.2f}"),
    "rss_mb": ("Peak memory (RSS)", "MB", "{:.0f}"),
    "export_secs": ("Export", "s", "{:.1f}"),
}
SHORT = {
    "page_warm_ms": "warm page",
    "page_cold_ms": "cold page",
    "index_secs": "index build",
    "idle_cpu_pct": "idle CPU",
    "rss_mb": "peak memory",
    "export_secs": "export",
}
# Schwarzwald-style calm tones: fir green and clay for bars, text and budget line inherit.
OK_LIGHT, OVER_LIGHT, OK_DARK, OVER_DARK = "#5f8a6e", "#b5654a", "#8fb89c", "#d98b72"


def ms(x):
    return f"{x / 1000:.2f} s" if x >= 1000 else f"{x:.0f} ms"


def num(key, v):
    return METRICS[key][2].format(v)


def fmt(key, v):
    return f"{num(key, v)} {METRICS[key][1]}"


def summary(name, r):
    over = [(k, b) for k, b in r["budget"].items() if b["status"] != "pass"]
    n = len(r["budget"])
    L = [
        f"### {name}",
        "",
        f"{r['class']} repository, pinned at `{r['sha'][:10]}`. "
        + (
            f"{n - len(over)} of {n} metrics are within budget."
            if over
            else f"All {n} metrics are within budget."
        )
        + " Against budget: "
        + ", ".join(
            f"{SHORT[k]} {fmt(k, b['value'])} of {b['limit']} {METRICS[k][1]}"
            for k, b in r["budget"].items()
            if k in METRICS
        )
        + ".",
        "",
    ]
    for k, b in over:
        L.append(
            f"- **Over budget:** {SHORT[k]} is {fmt(k, b['value'])} against a limit of "
            f"{b['limit']} {METRICS[k][1]} ({b['value'] / b['limit']:.1f}x)."
        )
    if over:
        L.append("")
    return L


def chart(key, repos):
    title, unit, _ = METRICS[key]
    rows = [(n, r["budget"][key]) for n, r in repos.items() if key in r["budget"]]
    if not rows:
        return []
    top = max(max(b["value"], b["limit"]) for _, b in rows) * 1.08 or 1
    lx, bx, bw, rh, pad = 8, 84, 380, 34, 30
    w, h = 640, pad + rh * len(rows) + 22
    s = [
        f'<svg xmlns="http://www.w3.org/2000/svg" role="img" viewBox="0 0 {w} {h}" width="{w}" '
        f'aria-label="{escape(title)} per repository against its budget">',
        f"<title>{escape(title)} ({unit}) against budget</title>",
        "<style>.ok{fill:%s}.over{fill:%s}@media(prefers-color-scheme:dark){.ok{fill:%s}.over{fill:%s}}</style>"
        % (OK_LIGHT, OVER_LIGHT, OK_DARK, OVER_DARK),
        f'<text x="{lx}" y="16" fill="currentColor" font-family="sans-serif" font-size="13" '
        f'font-weight="600">{escape(title)} ({unit})</text>',
    ]
    for i, (n, b) in enumerate(rows):
        y = pad + i * rh
        cls = "ok" if b["status"] == "pass" else "over"
        bar = max(bw * b["value"] / top, 1.5)
        lim = bw * b["limit"] / top
        s += [
            f'<text x="{lx}" y="{y + 17}" fill="currentColor" font-family="sans-serif" font-size="12">{escape(n)}</text>',
            f'<rect class="{cls}" x="{bx}" y="{y + 4}" width="{bar:.1f}" height="18" rx="2" fill="{OK_LIGHT if cls == "ok" else OVER_LIGHT}"/>',
            f'<line x1="{bx + lim:.1f}" y1="{y}" x2="{bx + lim:.1f}" y2="{y + 26}" stroke="currentColor" '
            f'stroke-width="1.5" stroke-dasharray="3 2"/>',
            f'<text x="{bx + bw + 14}" y="{y + 17}" fill="currentColor" font-family="sans-serif" font-size="12">'
            f"{num(key, b['value'])} of {b['limit']}{' (over)' if cls == 'over' else ''}</text>",
        ]
    s.append(
        f'<text x="{bx}" y="{h - 6}" fill="currentColor" font-family="sans-serif" font-size="11" '
        f'opacity="0.7">dashed line: budget</text></svg>'
    )
    return ["\n".join(s), ""]


def block(cur, old):
    m = cur["machine"]
    L = [
        f"Run {cur['when']} on {m['host']}: {m['cpu']}, {m['cpus']} cores, {m['ram_gb']} GB, {m['os']}. "
        f"{m['git']}, {m['gitterrace']} (repo at `{m['gitterrace_sha'][:10]}`). "
        f"{cur['runs']} warm runs per page, {cur['idle_secs']} s idle windows.",
        "",
    ]
    for n, r in cur["repos"].items():
        L += summary(n, r)
    L += [
        "| Repo | Class | Pin | Index | Peak RSS | Cache | RSS after pages | Idle CPU | Idle CPU with /events | Export |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for n, r in cur["repos"].items():
        ex = (
            f"{r['export']['secs']:.1f} s, {r['export']['out_mb']:.0f} MB"
            if "export" in r
            else "not run"
        )
        L.append(
            f"| {n} | {r['class']} | `{r['sha'][:10]}` | {r['index']['secs']:.1f} s | {r['index']['peak_rss_mb']:.0f} MB | "
            f"{r['index'].get('cache_mb', 0):.0f} MB | {r['memory']['rss_after_pages_mb']:.0f} MB | "
            f"{r['idle']['cpu_pct']:.2f}% | {r['idle']['cpu_pct_events_client']:.2f}% | {ex} |"
        )
    L.append("")
    for key in METRICS:
        L += chart(key, cur["repos"])
    for n, r in cur["repos"].items():
        prev = (old or {}).get("repos", {}).get(n)
        head = (
            "| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |"
            + (" Previous warm |" if prev else "")
        )
        L += [
            f"<details><summary>{n}: every page</summary>",
            "",
            f"Probes: `{r['probe']['blob']}` (largest text file), commit `{r['probe']['commit'][:10]}`.",
            "",
            head,
            "| --- | --- | --- | --- | --- | --- | --- |" + (" --- |" if prev else ""),
        ]
        for label, p in r["pages"].items():
            w = p["warm_total_ms"]
            row = (
                f"| {label} | {ms(p['cold']['total_ms'])} | {ms(p['warm_ttfb_ms']['median'])} | {ms(w['median'])} | "
                f"{ms(w['min'])} | {ms(w['max'])} | {p['bytes']} |"
            )
            if prev and label in prev["pages"]:
                row += f" {ms(prev['pages'][label]['warm_total_ms']['median'])} |"
            elif prev:
                row += " - |"
            L.append(row)
        L += ["", "| Budget | Value | Limit | Status |", "| --- | --- | --- | --- |"]
        for k, b in r["budget"].items():
            L.append(f"| {k} | {b['value']:.1f} | {b['limit']} | {b['status']} |")
        L += ["", "</details>", ""]
    return "\n".join(L).rstrip("\n") + "\n"


# Prose width of every tracked page; scripts/markdownlint/prose-length.cjs enforces it, and
# xtask's out::wrap_markdown applies the same rule to the Rust-generated blocks.
WIDTH = 100


def wrap(md):
    """Wrap prose lines at WIDTH; tables, fences, headings, lists, HTML and SVG stay as written."""
    out, fence = [], False
    for line in md.split("\n"):
        t = line.lstrip()
        if t.startswith("```"):
            fence = not fence
        if (
            fence
            or t.startswith(("```", "|", "#", "<", "- ", "* ", ">"))
            or len(line) <= WIDTH
        ):
            out.append(line)
            continue
        # A code span never breaks: protect its spaces while wrapping.
        safe = re.sub(r"`[^`]*`", lambda m: m.group(0).replace(" ", "\0"), line)
        lines = textwrap.wrap(
            safe, WIDTH, break_long_words=False, break_on_hyphens=False
        )
        out.extend(x.replace("\0", " ") for x in lines)
    return "\n".join(out)


def render():
    files = sorted((ROOT / "bench/results").glob("*.json"))
    if not files:
        sys.exit("no results in bench/results; run `make bench` first")
    cur = json.loads(files[-1].read_text())
    old = json.loads(files[-2].read_text()) if len(files) > 1 else None
    return wrap(block(cur, old)), files[-1].name


def main():
    body, name = render()
    if "--print" in sys.argv:
        sys.stdout.write(body)
        return
    text = DOC.read_text()
    pat = re.escape(BEGIN) + ".*?" + re.escape(END)
    if not re.search(pat, text, flags=re.S):
        sys.exit(f"{DOC} has no generated results block")
    new = re.sub(pat, lambda _: f"{BEGIN}\n{body}\n{END}", text, flags=re.S)
    if "--check" in sys.argv:
        if new != text:
            sys.exit("docs/benchmarks.md#results is stale, run `make bench-report`")
        print("docs/benchmarks.md#results: up to date")
        return
    DOC.write_text(new)
    print(f"updated {DOC.relative_to(ROOT)} from {name}")


main()
