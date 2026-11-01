#!/usr/bin/env python3
"""Turn the newest bench/results/*.json into the generated block of docs/benchmarks.md.

Run by `make bench-report`. Keeps the previous run's median warm page time per repo as a
comparison column when an older results file exists. Stdlib only.
"""
import json, re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DOC = ROOT / "docs/benchmarks.md"
BEGIN, END = "<!-- BEGIN GENERATED: results -->", "<!-- END GENERATED: results -->"


def ms(x):
    return f"{x / 1000:.2f} s" if x >= 1000 else f"{x:.0f} ms"


def block(cur, old):
    m = cur["machine"]
    L = [f"Run {cur['when']} on {m['host']}: {m['cpu']}, {m['cpus']} cores, {m['ram_gb']} GB, {m['os']}. "
         f"{m['git']}, {m['gitterrace']} (repo at `{m['gitterrace_sha'][:10]}`). "
         f"{cur['runs']} warm runs per page, {cur['idle_secs']} s idle windows.", ""]
    L += ["| Repo | Class | Pin | Index | Peak RSS | Cache | RSS after pages | Idle CPU | Idle CPU with /events | Export |",
          "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"]
    for n, r in cur["repos"].items():
        ex = f"{r['export']['secs']:.1f} s, {r['export']['out_mb']:.0f} MB" if "export" in r else "not run"
        L.append(f"| {n} | {r['class']} | `{r['sha'][:10]}` | {r['index']['secs']:.1f} s | {r['index']['peak_rss_mb']:.0f} MB | "
                 f"{r['index'].get('cache_mb', 0):.0f} MB | {r['memory']['rss_after_pages_mb']:.0f} MB | "
                 f"{r['idle']['cpu_pct']:.2f}% | {r['idle']['cpu_pct_events_client']:.2f}% | {ex} |")
    L.append("")
    for n, r in cur["repos"].items():
        prev = (old or {}).get("repos", {}).get(n)
        head = "| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |" + (" Previous warm |" if prev else "")
        L += [f"#### {n}", "", f"Probes: `{r['probe']['blob']}` (largest text file), commit `{r['probe']['commit'][:10]}`.", "",
              head, "| --- | --- | --- | --- | --- | --- | --- |" + (" --- |" if prev else "")]
        for label, p in r["pages"].items():
            w = p["warm_total_ms"]
            row = (f"| {label} | {ms(p['cold']['total_ms'])} | {ms(p['warm_ttfb_ms']['median'])} | {ms(w['median'])} | "
                   f"{ms(w['min'])} | {ms(w['max'])} | {p['bytes']} |")
            if prev and label in prev["pages"]:
                row += f" {ms(prev['pages'][label]['warm_total_ms']['median'])} |"
            elif prev:
                row += " - |"
            L.append(row)
        L += ["", "| Budget | Value | Limit | Status |", "| --- | --- | --- | --- |"]
        for k, b in r["budget"].items():
            L.append(f"| {k} | {b['value']:.1f} | {b['limit']} | {b['status']} |")
        L.append("")
    return "\n".join(L)


def main():
    files = sorted((ROOT / "bench/results").glob("*.json"))
    if not files:
        sys.exit("no results in bench/results; run `make bench` first")
    cur = json.loads(files[-1].read_text())
    old = json.loads(files[-2].read_text()) if len(files) > 1 else None
    text = DOC.read_text()
    new = re.sub(re.escape(BEGIN) + ".*?" + re.escape(END), lambda _: f"{BEGIN}\n{block(cur, old)}\n{END}", text, flags=re.S)
    if new == text and BEGIN not in text:
        sys.exit(f"{DOC} has no generated results block")
    DOC.write_text(new)
    print(f"updated {DOC.relative_to(ROOT)} from {files[-1].name}")


main()
