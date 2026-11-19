#!/usr/bin/env python3
"""The docs/benchmarks.md block and the results folder READMEs for
scripts/bench/bench-report.py. Imported, not run.
"""

from __future__ import annotations

import importlib
import re
import subprocess
import sys
import textwrap
from pathlib import Path
from typing import Any

bc = importlib.import_module("bench-chart-lib")

Json = dict[str, Any]


# The doc block ------------------------------------------------------------------------------


def slowest_page(b: Json) -> str:
    """The page a page budget was set by, when the run named it."""
    return f" ({b['page']})" if "page" in b else ""


def spread(key: str, b: Json) -> str:
    return (
        f" ({bc.num(key, b['min'])} to {bc.num(key, b['max'])})" if "min" in b else ""
    )


def samples_note(r: Json) -> str:
    n = r.get("cold_samples")
    if not n:
        return "1 sample, before repeats"
    return "1 sample" if n == 1 else f"median of {n}"


# The control repository is gitterrace itself: its commits are never named on a page, the run
# date stands for them. Corpus repositories keep their upstream sha.
OWN = "control"


HEX7 = re.compile(r"[0-9a-f]{7}")
# A run whose commit is not known yet: it renders as the plain version until the field is filled.
PENDING = "pending"
# Runs from this date on must record the commit they measured (or PENDING).
COMMIT_FROM = "2026-11-09"


def version(m: Json) -> str:
    """`gitterrace v0.1.0-1161f1d`: the version and the build's recorded commit, never the dirty
    mark. A run with no recorded commit renders as the plain `gitterrace v0.1.0`."""
    base = re.sub(r"(v\d+\.\d+\.\d+)\S*", r"\1", m["gitterrace"])
    c = m.get("commit")
    return f"{base}-{c}" if isinstance(c, str) and HEX7.fullmatch(c) else base


def commit_errors(run: Json) -> list[str]:
    """A run from COMMIT_FROM on records the commit it measured as 7 hex (or PENDING), and the
    version it renders names that commit."""
    if run["when"][:10] < COMMIT_FROM:
        return []
    m, name = run["machine"], run["when"][:10]
    c = m.get("commit")
    if c == PENDING:
        return []
    if not isinstance(c, str) or not HEX7.fullmatch(c):
        return [f"{name}: machine.commit is {c!r}, not the 7-hex commit of the build"]
    if not version(m).endswith(f"-{c}"):
        return [f"{name}: the rendered version does not name commit {c}"]
    return []


def at(name: str, sha: str, when: str) -> str:
    return f"the run of {when[:10]}" if name == OWN else f"`{sha[:10]}`"


def summary(name: str, r: Json, when: str) -> list[str]:
    over = [(k, b) for k, b in r["budget"].items() if b["status"] != "pass"]
    n = len(r["budget"])
    L = [
        f"### {name}",
        "",
        f"{r['class']} repository, measured at {at(name, r['sha'], when)}. "
        + (
            f"{n - len(over)} of {n} metrics are within budget."
            if over
            else f"All {n} metrics are within budget."
        )
        + " Against budget: "
        + ", ".join(
            f"{bc.SHORT[k]} {bc.fmt(k, b['value'])}{spread(k, b)}{slowest_page(b)} of {b['limit']} "
            f"{bc.METRICS[k][1]}"
            for k, b in r["budget"].items()
            if k in bc.METRICS
        )
        + ".",
        "",
    ]
    idle = r.get("idle")
    if idle:
        L += [
            f"Idle CPU is the larger of two windows: {idle['cpu_pct']:.2f} % with no client and "
            f"{idle['cpu_pct_events_client']:.2f} % with one /events client.",
            "",
        ]
    for k, b in over:
        L.append(
            f"- **Over budget:** {bc.SHORT[k]} is {bc.fmt(k, b['value'])} against a limit of "
            f"{b['limit']} {bc.METRICS[k][1]} ({b['value'] / b['limit']:.1f}x)."
        )
    if over:
        L.append("")
    return L


# `export --commits` default: an export at it is not called bounded.
DEFAULT_EXPORT_COMMITS = 500
CAPPED_EXPORT = "stopped at the page cap"
FAILED_EXPORT = "failed"
HELD = "withheld (load)"


def export_failure(e: Json) -> str:
    """Why an export did not finish. The cap is a documented limit, any other error a failure.
    A run that kept no reason says only that it failed, never which."""
    return (
        CAPPED_EXPORT if "stopped at the cap" in e.get("error", "") else FAILED_EXPORT
    )


def export_cell(r: Json) -> str:
    """An export that did not finish (ok false) is never shown as a time."""
    e = r.get("export")
    if not e:
        return "not run"
    if e.get("ok") is False:
        return export_failure(e)
    # A bounded export (corpus export_commits) says so: it is never a full export. One timed
    # run (corpus export_samples) says so too, beside a median of several elsewhere.
    n = e.get("commits", DEFAULT_EXPORT_COMMITS)
    bound = f" ({n} commits)" if n != DEFAULT_EXPORT_COMMITS else ""
    once = ", 1 run" if r.get("cold_samples") and len(e.get("samples", [])) == 1 else ""
    return f"{e['secs']:.1f} s{bound}{once}, {e['out_mb']:.0f} MB"


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
    out = {**run, "repos": {}, "withheld": {}, "held": {}}
    for n, r in run["repos"].items():
        loud = loud_phases(r)
        if loud:
            out["withheld"][n] = loud
            out["held"][n] = r
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
    return "Measured on a quiet machine."


def change(key: str, old: float, new: float) -> str:
    if not old:
        return "no baseline"
    pct = (new - old) / old * 100
    if round(pct) == 0:
        return "unchanged"
    lower, higher = bc.DIRECTION[key]
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
            if k not in bc.METRICS:
                continue
            p = prev["budget"].get(k) if prev else None
            if p is None:
                L.append(
                    f"| {n} | {bc.SHORT[k]} | not run | {bc.fmt(k, b['value'])} | new in this run |"
                )
                continue
            ch = change(k, p["value"], b["value"])
            L.append(
                f"| {n} | {bc.SHORT[k]} | {bc.fmt(k, p['value'])} | "
                f"{bc.fmt(k, b['value'])} | {ch} |"
            )
    return [*L, ""]


def idle_cause(run: Json) -> list[str]:
    """Why idle CPU with a client differs between repos: only a working checkout is scanned."""
    local = {r["name"] for r in bc.corpus_repos() if r.get("local")}
    seen = [n for n in (*run["repos"], *run.get("held", {})) if n in local]
    bare = [n for n in (*run["repos"], *run.get("held", {})) if n not in local]
    if not seen or not bare:
        return []
    return [
        f"{', '.join(seen)} is a working checkout, so with a tab open the server also checks it for "
        f"uncommitted changes every second. {', '.join(bare)} are bare clones with no working tree, "
        "so they have nothing to check and idle lower.",
        "",
    ]


def idle_footprint(r: Json) -> str:
    fp = r["memory"].get("footprint_idle_mb")
    return "not measured" if fp is None else f"{fp:.0f} MB"


def block(cur: Json, old: Json | None, d: str) -> str:
    """The full run: per repo summary, the table, charts (d: their folder), comparison, pages."""
    L = [f"{cur['runs']} warm runs per page, {cur['idle_secs']} s idle windows.", ""]
    for n, r in cur["repos"].items():
        L += summary(n, r, cur["when"])
    L += [
        "| Repo | Class | Measured at | Index | Peak RSS | Cache | RSS after pages | Idle footprint, total | Idle CPU "
        "| Idle CPU with /events | Export | Cold samples |",
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |",
    ]
    for n, r in cur["repos"].items():
        ex = export_cell(r)
        L.append(
            f"| {n} | {r['class']} | {at(n, r['sha'], cur['when'])} | {r['index']['secs']:.1f} s | "
            f"{r['index']['peak_rss_mb']:.0f} MB | {r['index'].get('cache_mb', 0):.0f} MB | "
            f"{r['memory']['rss_after_pages_mb']:.0f} MB | {idle_footprint(r)} | "
            f"{r['idle']['cpu_pct']:.2f}% | "
            f"{r['idle']['cpu_pct_events_client']:.2f}% | {ex} | {samples_note(r)} |"
        )
    for n, r in cur.get("held", {}).items():
        L.append(
            f"| {n} | {r['class']} | {at(n, r['sha'], cur['when'])} | "
            + " | ".join([HELD] * 8)
            + f" | {samples_note(r)} |"
        )
    L.append("")
    L += bc.by_page(cur)
    L += idle_cause(cur)
    names = bc.charts(cur, old)
    for key in bc.METRICS:
        if f"{key}.svg" in names:
            L += bc.picture(cur, key, d)
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
            f"commit {at(n, r['probe']['commit'], cur['when'])}.",
            "",
            head,
            "| --- | --- | --- | --- | --- | --- | --- |" + (" --- |" if prev else ""),
        ]
        for label, p in r["pages"].items():
            w = p["warm_total_ms"]
            row = (
                f"| {label} | {bc.ms(p['cold']['total_ms'])} | "
                f"{bc.ms(p['warm_ttfb_ms']['median'])} | "
                f"{bc.ms(w['median'])} | {bc.ms(w['min'])} | {bc.ms(w['max'])} | {p['bytes']} |"
            )
            if prev and label in prev["pages"]:
                row += f" {bc.ms(prev['pages'][label]['warm_total_ms']['median'])} |"
            elif prev:
                row += " - |"
            L.append(row)
        L += ["", "| Budget | Value | Limit | Status |", "| --- | --- | --- | --- |"]
        for k, b in r["budget"].items():
            L.append(f"| {k} | {b['value']:.1f} | {b['limit']} | {b['status']} |")
        L += ["", "</details>", ""]
    return "\n".join(L).rstrip("\n") + "\n"


# Prose width of every tracked page. The kit's markdownlint prose-length rule enforces it, and
# xtask's out::wrap_markdown applies the same rule to the Rust-generated blocks.
WIDTH = 80


def wrap(md: str) -> str:
    """Wrap prose lines at WIDTH. Tables, fences, headings, lists and HTML stay as written."""
    out: list[str] = []
    fence = False
    for line in md.split("\n"):
        t = line.lstrip()
        if t.startswith("```"):
            fence = not fence
        item = t.startswith(("- ", "* ")) and line == t
        if (
            fence
            or t.startswith(("```", "|", "#", "<", ">"))
            or (t.startswith(("- ", "* ")) and not item)
            or len(line) <= WIDTH
        ):
            out.append(line)
            continue
        # A code span never breaks: protect its spaces while wrapping. A top-level list item
        # continues under its text with a 2-space hang, as Markdown reads it.
        safe = re.sub(r"`[^`]*`", lambda m: m.group(0).replace(" ", "\0"), line)
        lines = textwrap.wrap(
            safe,
            WIDTH,
            subsequent_indent="  " if item else "",
            break_long_words=False,
            break_on_hyphens=False,
        )
        out.extend(x.replace("\0", " ") for x in lines)
    return "\n".join(out)


# Publishing ---------------------------------------------------------------------------------


def published() -> list[Json]:
    """The kept runs, oldest first, normalized."""
    files = bc.runs()
    if not files:
        sys.exit("no results in tests/bench/results, run `make bench` first")
    return [bc.normalize(bc.load(f)) for f in files[-bc.KEEP :]]


def expected() -> dict[Path, str]:
    """Every file under tests/bench/results the kept runs should produce, with its content."""
    out: dict[Path, str] = {}
    raw = published()
    kept = [shown(r) for r in raw]
    for i, run in enumerate(kept):
        name = bc.run_name(run)
        out[bc.RESULTS / f"{name}.json"] = bc.dumps(raw[i])
        for f, svg in bc.charts(run, kept[i - 1] if i else None).items():
            out[bc.RESULTS / name / f] = svg
        out[bc.RESULTS / name / "README.md"] = folder_readme(run)
    out[bc.RESULTS / "README.md"] = results_readme(kept)
    return out


def header(run: Json) -> str:
    m = run["machine"]
    return (
        f"Run {run['when'][:10]} on {machine_line(m)}. {version(m)}, {m['git']}. "
        + load_note(run)
        + (f" {run['note']}" if run.get("note") else "")
    )


HEADLINE = ("index_secs", "page_warm_ms", "page_cold_ms", "rss_mb", "export_secs")


def folder_readme(run: Json) -> str:
    """What a run's chart folder shows when opened: its header and its charts."""
    L = [
        f"# Benchmark run {bc.run_name(run)}",
        "",
        header(run),
        "",
        *[x + "\n" for x in withheld_lines(run)],
        f"Numbers: [`../{bc.run_name(run)}.json`](../{bc.run_name(run)}.json). "
        "Method: [docs/benchmarks.md](../../../../docs/benchmarks.md).",
        "",
    ]
    for key in bc.METRICS:
        if any(key in r["budget"] for r in run["repos"].values()):
            L += bc.picture(run, key, ".")
    return wrap("\n".join(L).rstrip("\n") + "\n")


def results_table(cur: Json, old: Json | None) -> list[str]:
    """The newest run's headline numbers against their budgets, one row per repo and metric,
    with the change against the previous run when there is one."""
    L = [
        "| Repo | Metric | Value | Budget | Status |"
        + (" Change against previous |" if old else ""),
        "| --- | --- | --- | --- | --- |" + (" --- |" if old else ""),
    ]
    for n, r in cur["repos"].items():
        prev = (old or {}).get("repos", {}).get(n, {}).get("budget", {})
        for k in HEADLINE:
            b = r["budget"].get(k)
            if not b:
                if k == "export_secs" and r.get("export", {}).get("ok") is False:
                    why = export_failure(r["export"])
                    status = "capped" if why == CAPPED_EXPORT else "failed"
                    L.append(
                        f"| {n} | {bc.SHORT[k]} | {why} | - | {status} |"
                        + (" - |" if old else "")
                    )
                continue
            p = prev.get(k)
            ch = change(k, p["value"], b["value"]) if p else "new in this run"
            L.append(
                f"| {n} | {bc.SHORT[k]} | {bc.fmt(k, b['value'])}{spread(k, b)} | "
                f"{b['limit']} {bc.METRICS[k][1]} | {b['status']} |"
                + (f" {ch} |" if old else "")
            )
    for n in cur.get("held", {}):
        for k in HEADLINE:
            L.append(
                f"| {n} | {bc.SHORT[k]} | {HELD} | - | withheld |"
                + (" - |" if old else "")
            )
    return L


def results_readme(kept: list[Json]) -> str:
    """The landing page of tests/bench/results: every kept run, the newest headline, its charts."""
    cur = kept[-1]
    old = kept[-2] if len(kept) > 1 else None
    L = [
        "# Benchmark results",
        "",
        "Each benchmark run (`make bench`, with `LARGE=1` to add the largest repository) writes one "
        "`<date>-<machine>.json` here plus a folder with its charts, and "
        "`scripts/bench/bench-report.py` regenerates this page. The method, corpus and "
        "budgets are in "
        "[docs/benchmarks.md](../../../docs/benchmarks.md).",
        "",
        "| Date | Machine | gitterrace | Repos | Files |",
        "| --- | --- | --- | --- | --- |",
    ]
    for run in reversed(kept):
        m, n = run["machine"], bc.run_name(run)
        L.append(
            f"| {run['when'][:10]} | {m['cpu']}, {m['cpus']} cores, {m['ram_gb']:.0f} GB RAM, "
            f"{m['os']} | {version(m)} | "
            f"{', '.join(run['repos'])} | [json]({n}.json), [charts]({n}/) |"
        )
    L += [
        "",
        f"## Newest run, {cur['when'][:10]}",
        "",
        header(cur),
        "",
        *[x + "\n" for x in withheld_lines(cur)],
        *results_table(cur, old),
    ]
    L += ["", block(cur, old, bc.run_name(cur))]
    return wrap("\n".join(L).rstrip("\n") + "\n")


def render() -> str:
    """The results block of docs/benchmarks.md: the newest run's header and its table against
    the budgets. The charts and every page are drawn into tests/bench/results, untracked."""
    runs = published()
    cur = shown(runs[-1])
    old = shown(runs[-2]) if len(runs) > 1 else None
    L = [
        header(cur),
        "",
        *[x + "\n" for x in withheld_lines(cur)],
        *results_table(cur, old),
    ]
    return wrap("\n".join(L).strip("\n") + "\n")


def spliced(text: str, body: str) -> str:
    pat = re.escape(bc.BEGIN) + ".*?" + re.escape(bc.END)
    if not re.search(pat, text, flags=re.S):
        sys.exit(f"{bc.DOC} has no generated results block")
    return re.sub(pat, lambda _: f"{bc.BEGIN}\n{body}\n{bc.END}", text, flags=re.S)


def owned(p: Path) -> bool:
    """A file this report writes: run JSON, chart SVG or README."""
    return p.suffix in (".json", ".svg") or p.name == "README.md"


def tracked(p: Path) -> bool:
    """A file the repository keeps: the run JSON. Charts and READMEs are drawn from it."""
    return p.suffix == ".json"


def actual(root: Path = bc.RESULTS) -> set[Path]:
    """The report's own files under root. Other files are stray(), never read as text."""
    return {p for p in root.rglob("*") if p.is_file() and owned(p)}


def ignored(paths: list[Path]) -> set[Path]:
    """The paths git ignores (a Finder .DS_Store), so they are neither published nor flagged."""
    if not paths:
        return set()
    r = subprocess.run(
        ["git", "check-ignore", "--stdin"],
        cwd=bc.ROOT,
        input="\n".join(str(p) for p in paths),
        capture_output=True,
        text=True,
    )
    return {Path(x) for x in r.stdout.splitlines()}


def stray(root: Path = bc.RESULTS) -> list[str]:
    """Files under root the report does not own and git does not ignore."""
    other = [p for p in root.rglob("*") if p.is_file() and not owned(p)]
    skip = ignored(other)
    return [
        f"unexpected file: {p.relative_to(root.parent.parent)}"
        for p in sorted(other)
        if p not in skip
    ]
