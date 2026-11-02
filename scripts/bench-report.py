#!/usr/bin/env python3
"""Publish bench/results: normalize the run files, render their charts, write docs/benchmarks.md.

One run is bench/results/<date>-<machine>.json plus its charts in bench/results/<date>-<machine>/
(<metric>.svg and <metric>-dark.svg). The newest two runs are kept: the newest in full, the one
before it as the baseline of the comparison table. The machine label comes from the CPU, never
the hostname. Stdlib only, no PNG, no JS, no fonts. The charts are files, not inline SVG, since a
markdown sanitizer strips inline <svg> and <style>, and the doc links them through <picture>.

  scripts/bench-report.py                  normalize, render charts, prune, rewrite the doc
  scripts/bench-report.py --check          fail when anything above is stale, writes nothing
  scripts/bench-report.py --print          print the results block (what `cargo xtask` splices)
  scripts/bench-report.py --corpus         print the corpus list, one source link per repo
  scripts/bench-report.py --normalize F    rewrite one result file in the published format

--check also guards publication: no result file, chart or doc may carry this machine's hostname
or a "host" field, every result file is sorted and rounded, every corpus repo has a url and a
why, a local repo has no sha (it is measured at HEAD), a fetched clone holds its pin, no doc
has an inline <svg>, and the newest run comes from a clean build timed on a quiet machine.
"""

from __future__ import annotations

import json
import os
import platform
import re
import shutil
import subprocess
import sys
import textwrap
from html import escape
from pathlib import Path
from typing import Any

Json = dict[str, Any]

ROOT = Path(__file__).resolve().parent.parent
DOC = ROOT / "docs/benchmarks.md"
DOCS = ROOT / "docs"
CORPUS = ROOT / "bench/corpus.toml"
RESULTS = ROOT / "bench/results"
BEGIN, END = "<!-- BEGIN GENERATED: results -->", "<!-- END GENERATED: results -->"
# Result files are <date>-<machine label>.json. The label comes from the CPU, never the host.
NAME = re.compile(r"\d{4}-\d{2}-\d{2}-[a-z0-9-]+\.json")
# The newest run and its baseline. Older runs live in git history.
KEEP = 2
# Cold samples per repo, matching bench-run.py. corpus.toml lowers it per repo.
COLD_SAMPLES = 3
# Floats are rounded to this many decimals, so diffs between runs stay readable.
DECIMALS = 2

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


def corpus_repos() -> list[dict[str, str]]:
    """The [[repo]] tables of corpus.toml as flat string maps, in corpus order."""
    out: list[dict[str, str]] = []
    cur: dict[str, str] | None = None
    for line in CORPUS.read_text().splitlines():
        if line.strip() == "[[repo]]":
            cur = {}
            out.append(cur)
        elif line.startswith("["):
            cur = None
        elif cur is not None and "=" in line and not line.startswith("#"):
            k, v = (x.strip() for x in line.split("=", 1))
            cur[k] = v.strip('"')
    return out


def corpus() -> list[tuple[str, str, str]]:
    """(name, source page, why) per repo, from corpus.toml so the doc list can't drift."""
    repos = corpus_repos()
    for r in repos:
        if not r.get("url") or not r.get("why"):
            sys.exit(f"bench/corpus.toml: repo {r.get('name')} needs a url and a why")
        if r.get("local") and r.get("sha"):
            sys.exit(
                f"bench/corpus.toml: {r['name']} is local, measured at HEAD: drop its sha"
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
        n: repos[n]
        for n in sorted(repos, key=lambda n: (order.index(n) if n in order else 99, n))
    }
    return rounded(run)


def dumps(run: Json) -> str:
    """Sorted keys inside each repo, corpus order for the repos map itself."""
    repos = run["repos"]
    body = {k: v for k, v in run.items() if k != "repos"}
    text = json.dumps({**body, "repos": {}}, indent=1, sort_keys=True)
    inner = ",\n".join(
        f" {json.dumps(n)}: "
        + json.dumps(r, indent=1, sort_keys=True).replace("\n", "\n ")
        for n, r in repos.items()
    )
    return text.replace('"repos": {}', '"repos": {\n' + inner + "\n }") + "\n"


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
    "export_secs": ("faster", "slower"),
    "idle_cpu_pct": ("less CPU", "more CPU"),
    "rss_mb": ("less memory", "more memory"),
}


def chart(key: str, repos: Json, scheme: str, prev: Json | None = None) -> str | None:
    """Bars against the budget line. With prev, a hollow marker shows the previous run's value."""
    title, unit, _ = METRICS[key]
    ok, over, ink = PALETTE[scheme]
    rows = [(n, r["budget"][key]) for n, r in repos.items() if key in r["budget"]]
    if not rows:
        return None
    before = {
        n: r["budget"][key]["value"]
        for n, r in (prev or {}).items()
        if key in r.get("budget", {})
    }
    top = (
        max(max(b["value"], b["limit"], before.get(n, 0)) for n, b in rows) * 1.08 or 1
    )
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
    d = f"../bench/results/{run_name(run)}" if d is None else d
    title = METRICS[key][0]
    return [
        "<picture>",
        f'<source media="(prefers-color-scheme: dark)" srcset="{d}/{key}-dark.svg">',
        f'<img alt="{escape(title)} per repository against its budget" src="{d}/{key}.svg">',
        "</picture>",
        "",
    ]


# The doc block ------------------------------------------------------------------------------


def spread(key: str, b: Json) -> str:
    return f" ({num(key, b['min'])} to {num(key, b['max'])})" if "min" in b else ""


def samples_note(r: Json) -> str:
    n = r.get("cold_samples")
    if not n:
        return "1 sample, before repeats"
    return "1 sample" if n == 1 else f"median of {n}"


def summary(name: str, r: Json) -> list[str]:
    over = [(k, b) for k, b in r["budget"].items() if b["status"] != "pass"]
    n = len(r["budget"])
    L = [
        f"### {name}",
        "",
        f"{r['class']} repository, measured at `{r['sha'][:10]}`. "
        + (
            f"{n - len(over)} of {n} metrics are within budget."
            if over
            else f"All {n} metrics are within budget."
        )
        + " Against budget: "
        + ", ".join(
            f"{SHORT[k]} {fmt(k, b['value'])}{spread(k, b)} of {b['limit']} {METRICS[k][1]}"
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


def machine_line(m: Json) -> str:
    """Every machine stat except its name: CPU, cores, RAM, disk, OS, arch."""
    parts = [m["cpu"], f"{m['cpus']} cores", f"{m['ram_gb']:.0f} GB RAM"]
    if m.get("disk"):
        parts.append(f"{m['disk']} disk")
    parts.append(f"{m['os']} ({m['arch']})")
    return ", ".join(parts)


QUIET_NOTE = "not measured on a quiet machine"


def loud_phases(r: Json) -> list[str]:
    """Phases whose load, sampled while they ran, reached the limit.

    Runs before the sampler have no load.sampled: unknown, shown as before."""
    load = r.get("load") or {}
    return [p for p, s in load.get("sampled", {}).items() if s["max"] >= load["limit"]]


def shown(run: Json) -> Json:
    """The publishable view of a run: a repo with a loud phase is withheld, never shown."""
    out = {**run, "repos": {}, "withheld": {}}
    for n, r in run["repos"].items():
        loud = loud_phases(r)
        if loud:
            out["withheld"][n] = loud
        else:
            out["repos"][n] = r
    return out


def withheld_lines(run: Json) -> list[str]:
    return [
        f"- **{n}:** {QUIET_NOTE} (other processes used {run_limit(run)} or more cores during "
        f"{', '.join(phases)}), so its numbers are withheld."
        for n, phases in run.get("withheld", {}).items()
    ]


def run_limit(run: Json) -> str:
    limits = {r["load"]["limit"] for r in run["repos"].values() if r.get("load")}
    return f"{max(limits):g}" if limits else "unknown"


def load_note(run: Json) -> str:
    limits = {r["load"]["limit"] for r in run["repos"].values() if r.get("load")}
    if not limits:
        return "This run predates the quiet-machine guard and recorded no load."
    return (
        "Every timed phase started with the 1-minute load average under "
        f"{max(limits):g}."
    )


def change(key: str, old: float, new: float) -> str:
    if not old:
        return "no baseline"
    pct = (new - old) / old * 100
    if round(pct) == 0:
        return "unchanged"
    lower, higher = DIRECTION[key]
    return f"{abs(pct):.0f}% {lower if pct < 0 else higher}"


def comparison(cur: Json, old: Json) -> list[str]:
    L = [
        f"### Compared with {old['when'][:10]}",
        "",
        f"The previous run, on {machine_line(old['machine'])}. "
        + load_note(old)
        + " Its charts stay beside its result file. On the charts above, a hollow marker shows its"
        " value.",
        "",
        f"| Repo | Metric | {old['when'][:10]} | {cur['when'][:10]} | Change |",
        "| --- | --- | --- | --- | --- |",
    ]
    for n, r in cur["repos"].items():
        prev = old["repos"].get(n)
        for k, b in r["budget"].items():
            if k not in METRICS:
                continue
            p = prev["budget"].get(k) if prev else None
            if p is None:
                L.append(
                    f"| {n} | {SHORT[k]} | not run | {fmt(k, b['value'])} | new in this run |"
                )
                continue
            ch = change(k, p["value"], b["value"])
            L.append(
                f"| {n} | {SHORT[k]} | {fmt(k, p['value'])} | {fmt(k, b['value'])} | {ch} |"
            )
    return [*L, ""]


def block(cur: Json, old: Json | None) -> str:
    m = cur["machine"]
    L = [
        f"Run {cur['when'][:10]} on {machine_line(m)}. "
        f"{m['git']}, {m['gitterrace']} (repo at `{m['gitterrace_sha'][:10]}`). "
        f"{cur['runs']} warm runs per page, {cur['idle_secs']} s idle windows. "
        + load_note(cur),
        "",
    ]
    gone = withheld_lines(cur)
    if gone:
        L += [*gone, ""]
    for n, r in cur["repos"].items():
        L += summary(n, r)
    L += [
        "| Repo | Class | Measured at | Index | Peak RSS | Cache | RSS after pages | Idle CPU "
        "| Idle CPU with /events | Export | Cold samples |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for n, r in cur["repos"].items():
        ex = (
            f"{r['export']['secs']:.1f} s, {r['export']['out_mb']:.0f} MB"
            if "export" in r
            else "not run"
        )
        L.append(
            f"| {n} | {r['class']} | `{r['sha'][:10]}` | {r['index']['secs']:.1f} s | "
            f"{r['index']['peak_rss_mb']:.0f} MB | {r['index'].get('cache_mb', 0):.0f} MB | "
            f"{r['memory']['rss_after_pages_mb']:.0f} MB | {r['idle']['cpu_pct']:.2f}% | "
            f"{r['idle']['cpu_pct_events_client']:.2f}% | {ex} | {samples_note(r)} |"
        )
    L.append("")
    names = charts(cur, old)
    for key in METRICS:
        if f"{key}.svg" in names:
            L += picture(cur, key)
    if old:
        L += comparison(cur, old)
    for n, r in cur["repos"].items():
        prev = (old or {}).get("repos", {}).get(n)
        head = (
            "| Page | Cold | Warm TTFB | Warm full (median) | min | max | Bytes |"
            + (" Previous warm |" if prev else "")
        )
        L += [
            f"<details><summary>{n}: every page</summary>",
            "",
            f"Probes: `{r['probe']['blob']}` (largest text file), "
            f"commit `{r['probe']['commit'][:10]}`.",
            "",
            head,
            "| --- | --- | --- | --- | --- | --- | --- |" + (" --- |" if prev else ""),
        ]
        for label, p in r["pages"].items():
            w = p["warm_total_ms"]
            row = (
                f"| {label} | {ms(p['cold']['total_ms'])} | {ms(p['warm_ttfb_ms']['median'])} | "
                f"{ms(w['median'])} | {ms(w['min'])} | {ms(w['max'])} | {p['bytes']} |"
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


# Prose width of every tracked page. scripts/markdownlint/prose-length.cjs enforces it, and
# xtask's out::wrap_markdown applies the same rule to the Rust-generated blocks.
WIDTH = 100


def wrap(md: str) -> str:
    """Wrap prose lines at WIDTH. Tables, fences, headings, lists and HTML stay as written."""
    out: list[str] = []
    fence = False
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


# Publishing ---------------------------------------------------------------------------------


def published() -> list[Json]:
    """The kept runs, oldest first, normalized."""
    files = runs()
    if not files:
        sys.exit("no results in bench/results, run `make bench` first")
    return [normalize(load(f)) for f in files[-KEEP:]]


def expected() -> dict[Path, str]:
    """Every file under bench/results the kept runs should produce, with its content."""
    out: dict[Path, str] = {}
    raw = published()
    kept = [shown(r) for r in raw]
    for i, run in enumerate(kept):
        name = run_name(run)
        out[RESULTS / f"{name}.json"] = dumps(raw[i])
        for f, svg in charts(run, kept[i - 1] if i else None).items():
            out[RESULTS / name / f] = svg
        out[RESULTS / name / "README.md"] = folder_readme(run)
    out[RESULTS / "README.md"] = results_readme(kept)
    return out


def header(run: Json) -> str:
    m = run["machine"]
    return (
        f"Run {run['when'][:10]} on {machine_line(m)}. {m['gitterrace']} "
        f"(repo at `{m['gitterrace_sha'][:10]}`), {m['git']}. " + load_note(run)
    )


HEADLINE = ("index_secs", "page_warm_ms", "page_cold_ms", "rss_mb", "export_secs")


def folder_readme(run: Json) -> str:
    """What a run's chart folder shows when opened: its header and its charts."""
    L = [
        f"# Benchmark run {run_name(run)}",
        "",
        header(run),
        "",
        *[x + "\n" for x in withheld_lines(run)],
        f"Numbers: [`../{run_name(run)}.json`](../{run_name(run)}.json). "
        "Method: [docs/benchmarks.md](../../../docs/benchmarks.md).",
        "",
    ]
    for key in METRICS:
        if any(key in r["budget"] for r in run["repos"].values()):
            L += picture(run, key, ".")
    return wrap("\n".join(L).rstrip("\n") + "\n")


def results_readme(kept: list[Json]) -> str:
    """The landing page of bench/results: every kept run, the newest headline, its charts."""
    cur = kept[-1]
    old = kept[-2] if len(kept) > 1 else None
    L = [
        "# Benchmark results",
        "",
        "Each run of `make bench LARGE=1` writes one `<date>-<machine>.json` here plus a folder with "
        "its charts, and `scripts/bench-report.py` regenerates this page. The method, corpus and "
        "budgets are in [docs/benchmarks.md](../../docs/benchmarks.md).",
        "",
        "| Date | Machine | gitterrace | Repos | Files |",
        "| --- | --- | --- | --- | --- |",
    ]
    for run in reversed(kept):
        m, n = run["machine"], run_name(run)
        L.append(
            f"| {run['when'][:10]} | {m['cpu']}, {m['cpus']} cores, {m['ram_gb']:.0f} GB RAM, "
            f"{m['os']} | {m['gitterrace']} `{m['gitterrace_sha'][:10]}` | "
            f"{', '.join(run['repos'])} | [json]({n}.json), [charts]({n}/) |"
        )
    L += [
        "",
        f"## Newest run, {cur['when'][:10]}",
        "",
        header(cur),
        "",
        *[x + "\n" for x in withheld_lines(cur)],
        "| Repo | Metric | Value | Budget | Status | Change against previous |",
        "| --- | --- | --- | --- | --- | --- |",
    ]
    for n, r in cur["repos"].items():
        prev = (old or {}).get("repos", {}).get(n, {}).get("budget", {})
        for k in HEADLINE:
            b = r["budget"].get(k)
            if not b:
                continue
            p = prev.get(k)
            ch = (
                change(k, p["value"], b["value"])
                if p
                else ("new in this run" if old else "first kept run")
            )
            L.append(
                f"| {n} | {SHORT[k]} | {fmt(k, b['value'])}{spread(k, b)} | "
                f"{b['limit']} {METRICS[k][1]} | {b['status']} | {ch} |"
            )
    L.append("")
    for key in METRICS:
        if any(key in r["budget"] for r in cur["repos"].values()):
            L += picture(cur, key, run_name(cur))
    return wrap("\n".join(L).rstrip("\n") + "\n")


def render() -> str:
    kept = [shown(r) for r in published()]
    return wrap(block(kept[-1], kept[-2] if len(kept) > 1 else None))


def spliced(text: str, body: str) -> str:
    pat = re.escape(BEGIN) + ".*?" + re.escape(END)
    if not re.search(pat, text, flags=re.S):
        sys.exit(f"{DOC} has no generated results block")
    return re.sub(pat, lambda _: f"{BEGIN}\n{body}\n{END}", text, flags=re.S)


def owned(p: Path) -> bool:
    """A file this report writes: run JSON, chart SVG or README."""
    return p.suffix in (".json", ".svg") or p.name == "README.md"


def actual(root: Path = RESULTS) -> set[Path]:
    """The report's own files under root. Other files are stray(), never read as text."""
    return {p for p in root.rglob("*") if p.is_file() and owned(p)}


def ignored(paths: list[Path]) -> set[Path]:
    """The paths git ignores (a Finder .DS_Store), so they are neither published nor flagged."""
    if not paths:
        return set()
    r = subprocess.run(
        ["git", "check-ignore", "--stdin"],
        cwd=ROOT,
        input="\n".join(str(p) for p in paths),
        capture_output=True,
        text=True,
    )
    return {Path(x) for x in r.stdout.splitlines()}


def stray(root: Path = RESULTS) -> list[str]:
    """Files under root the report does not own and git does not ignore."""
    other = [p for p in root.rglob("*") if p.is_file() and not owned(p)]
    skip = ignored(other)
    return [
        f"unexpected file: {p.relative_to(root.parent.parent)}"
        for p in sorted(other)
        if p not in skip
    ]


def self_test() -> None:
    """A stray binary file gives the clean message, not a decode crash."""
    import tempfile

    with tempfile.TemporaryDirectory() as d:
        bad = Path(d) / "sub" / "junk.bin"
        bad.parent.mkdir()
        bad.write_bytes(b"\x00\x80\xff not utf-8")
        (Path(d) / "a.json").write_text("{}")
        got = stray(Path(d))
        if got != [
            f"unexpected file: {bad.relative_to(Path(d).parent.parent)}"
        ] or actual(Path(d)) != {Path(d) / "a.json"}:
            sys.exit(f"bench-report self-test failed: {got}")
    loud_test()
    print("bench-report self-test: ok")


def loud_test() -> None:
    """A repo whose sampled load reached the limit mid-phase is withheld from the doc."""

    def repo(peak: float | None) -> Json:
        load: Json = {"limit": 5.0, "phases": {"index": 1.0}}
        if peak is not None:
            load["sampled"] = {"index": {"max": peak, "mean": 2.0, "n": 3}}
        return {"load": load}

    view = shown({"repos": {"quiet": repo(4.9), "loud": repo(41.0), "old": repo(None)}})
    if list(view["repos"]) != ["quiet", "old"] or view["withheld"] != {
        "loud": ["index"]
    }:
        sys.exit(f"bench-report self-test failed: loud phase published: {view}")
    if QUIET_NOTE not in "".join(withheld_lines(view)):
        sys.exit("bench-report self-test failed: withheld repo not named")


# Guards -------------------------------------------------------------------------------------


def pins() -> list[str]:
    """A clone that exists must hold its pin. Clones are optional, so a missing one passes."""
    base = Path(
        os.environ.get("GITTERRACE_BENCH_DIR", Path.home() / ".cache/gitterrace-bench")
    )
    errs: list[str] = []
    for r in corpus_repos():
        d = base / f"{r['name']}.git"
        if r.get("sha") and (d / "bench-ready").exists():
            ok = subprocess.run(
                ["git", "cat-file", "-e", f"{r['sha']}^{{commit}}"],
                cwd=d,
                capture_output=True,
            )
            if ok.returncode:
                errs.append(f"{r['name']}: pin {r['sha'][:10]} missing in {d}")
    return errs


def newest_ok(run: Json) -> list[str]:
    """The newest run must come from a clean build, timed under its load limit."""
    errs: list[str] = []
    if "dirty" in run["machine"].get("gitterrace", ""):
        errs.append(f"{run_name(run)}: measured a -dirty build")
    for n, r in run["repos"].items():
        load = r.get("load")
        if not load:
            errs.append(f"{run_name(run)}: {n} recorded no load")
            continue
        for phase, v in load["phases"].items():
            if phase != "background" and v >= load["limit"]:
                errs.append(f"{n}: {phase} timed at load {v} (limit {load['limit']})")
    return errs


def cold_samples(run: Json) -> list[str]:
    """Cold metrics need COLD_SAMPLES samples, or the count corpus.toml allows for the repo."""
    allowed = {
        r["name"]: int(r.get("cold_samples", COLD_SAMPLES)) for r in corpus_repos()
    }
    return [
        f"{run_name(run)}: {n} has {r.get('cold_samples', 1)} cold sample(s), "
        f"needs {allowed.get(n, COLD_SAMPLES)}"
        for n, r in run["repos"].items()
        if r.get("cold_samples", 1) < allowed.get(n, COLD_SAMPLES)
    ]


def no_inline_svg() -> list[str]:
    return [
        f"{p.relative_to(ROOT)}: inline <svg>, link a file through <picture> instead"
        for p in sorted(DOCS.rglob("*.md"))
        if re.search(r"<svg[\s>]", p.read_text())
    ]


def no_hostname(paths: list[Path]) -> list[str]:
    host = platform.node().split(".")[0].lower()
    errs: list[str] = []
    for p in paths:
        text = p.read_text().lower()
        if p.suffix == ".json" and '"host"' in text:
            errs.append(f"{p.relative_to(ROOT)}: has a host field")
        if host and re.search(rf"\b{re.escape(host)}\b", p.name.lower() + "\n" + text):
            errs.append(f"{p.relative_to(ROOT)}: contains the hostname")
    return errs


def check(text: str, new: str) -> None:
    want = expected()
    newest = published()[-1]
    errs = pins() + no_inline_svg() + newest_ok(newest) + cold_samples(newest)
    errs += [
        f"{p.name}: name is not <date>-<machine>.json"
        for p in runs()
        if not NAME.fullmatch(p.name)
    ]
    errs += stray()
    errs += no_hostname(sorted(actual()) + [DOC])
    for p, body in want.items():
        if not p.exists() or p.read_text() != body:
            errs.append(f"{p.relative_to(ROOT)}: stale or missing")
    errs += [
        f"{p.relative_to(ROOT)}: left over from an older run"
        for p in actual() - set(want)
    ]
    if new != text:
        errs.append("docs/benchmarks.md#results is stale")
    if errs:
        sys.exit("\n".join(errs) + "\nrun `make bench-report`")
    print("bench/results and docs/benchmarks.md: up to date")


def write() -> None:
    """Normalize the kept runs, render their charts, drop older runs, rewrite the doc."""
    files = runs()
    want = expected()
    for p in files[:-KEEP]:
        p.unlink()
    for p in files[-KEEP:]:
        if p not in want:
            p.unlink()
    for d in RESULTS.iterdir():
        if d.is_dir() and not any(p.parent == d for p in want):
            shutil.rmtree(d)
    for p in actual() - set(want):
        p.unlink()
    for p, body in want.items():
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(body)
    DOC.write_text(spliced(DOC.read_text(), render()))
    print(
        f"updated {DOC.relative_to(ROOT)} and bench/results from {len(published())} run(s)"
    )


def main() -> None:
    args = sys.argv[1:]
    if "--corpus" in args:
        sys.stdout.write(corpus_list())
        return
    if "--self-test" in args:
        self_test()
        return
    if "--normalize" in args:
        src = Path(args[args.index("--normalize") + 1])
        run = normalize(load(src))
        dst = src.with_name(f"{run_name(run)}.json")
        dst.write_text(dumps(run))
        if dst != src:
            src.unlink()
        print(f"wrote {dst}")
        return
    corpus()
    body = render()
    if "--print" in args:
        sys.stdout.write(body)
        return
    text = DOC.read_text()
    new = spliced(text, body)
    if "--check" in args:
        check(text, new)
        return
    write()


if __name__ == "__main__":
    main()
