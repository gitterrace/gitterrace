#!/usr/bin/env python3
"""Publish tests/bench/results: normalize the run files, render their charts, write the doc.

One run is tests/bench/results/<date>-<machine>.json plus its charts in
tests/bench/results/<date>-<machine>/
(<metric>.svg and <metric>-dark.svg). The newest two runs are kept: the newest in full, the one
before it as the baseline of the comparison table. The machine label comes from the CPU, never
the hostname. Stdlib only, no PNG, no JS, no fonts. The charts are files, not inline SVG, since a
markdown sanitizer strips inline <svg> and <style>, and the doc links them through <picture>.

  scripts/bench/bench-report.py                  normalize, render charts, prune, rewrite the doc
  scripts/bench/bench-report.py --check          fail when anything above is stale, writes nothing
  scripts/bench/bench-report.py --print          print the short results block that
                                                 `cargo xtask` splices
  scripts/bench/bench-report.py --corpus         print the corpus list, one source link per repo
  scripts/bench/bench-report.py --normalize F    rewrite one result file in the published format

--check also guards publication: no result file, chart or doc may carry this machine's hostname
or a "host" field, every result file is sorted and rounded, every corpus repo has a url and a
why, a local repo has no sha (it is measured at HEAD), a fetched clone holds its pin, no doc
has an inline <svg>, and the newest run comes from a clean build timed on a quiet machine.
"""

from __future__ import annotations

import importlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

bc = importlib.import_module("bench-chart-lib")
bd = importlib.import_module("bench-doc-lib")
bl = importlib.import_module("bench-lib")
bq = importlib.import_module("bench-quiet-lib")

Json = dict[str, Any]
ROOT = bc.ROOT
DOC = bc.DOC
DOCS = bc.DOCS
RESULTS = bc.RESULTS
NAME = bc.NAME
KEEP = bc.KEEP
COLD_SAMPLES = bc.COLD_SAMPLES
corpus_repos = bc.corpus_repos
corpus = bc.corpus
corpus_list = bc.corpus_list
normalize = bc.normalize
dumps = bc.dumps
run_name = bc.run_name
runs = bc.runs
load = bc.load
charts = bc.charts
picture = bc.picture
QUIET_NOTE = bd.QUIET_NOTE
shown = bd.shown
withheld_lines = bd.withheld_lines
published = bd.published
expected = bd.expected
render = bd.render
spliced = bd.spliced
actual = bd.actual
stray = bd.stray
_own: set[str] = set()


def own_commits() -> set[str]:
    """Every commit of this repository, read once."""
    if not _own:
        _own.update(bl.own_shas(ROOT))
    return _own


def gate_test() -> None:
    """Every labelled metric names its fields, and each of them alone can fail the gate."""
    br = importlib.import_module("bench-run")
    for key in bc.SOURCES:
        if key not in bc.METRICS:
            sys.exit(f"bench-report self-test failed: {key} has sources but no label")
    pages = {"warm_total_ms": {"median": 1}, "cold": {"total_ms": 1}}
    for key, fields in bc.SOURCES.items():
        for hot in fields:
            res: Json = {
                "class": "c",
                "pages": {"p": pages},
                "index": {"secs": 1, "peak_rss_mb": 1, "peak_footprint_mb": 1},
                "idle": {"cpu_pct": 0.1, "cpu_pct_events_client": 0.1},
                "memory": {"rss_after_pages_mb": 1, "footprint_after_pages_mb": 1},
            }
            top, leaf = hot.split(".")
            res[top][leaf] = 100
            got = br.judge(res, {"c": {key: 50}}).get(key, {}).get("status")
            if got != "over":
                sys.exit(
                    f"bench-report self-test failed: {hot} over budget reads {got}"
                )


# What the memory gate reads, see bench-chart-lib SOURCES.
MEASURE = bl.footprint_mb
CHILD = """
import mmap, sys, time
path, heap = sys.argv[1], int(sys.argv[2])
with open(path, "rb") as f:
    m = mmap.mmap(f.fileno(), 0, access=mmap.ACCESS_READ)
    s = sum(m[i] for i in range(0, len(m), 4096))
    buf = bytearray(heap)
    for i in range(0, heap, 4096):
        buf[i] = 1
    print("ready", s, flush=True)
    time.sleep(60)
"""


def memory_measure_test() -> None:
    """The memory gate's measure: a process holding a large mapped file and a small heap is
    under the limit, one with a large heap is over. Plain RSS would fail the first."""
    import subprocess
    import tempfile

    limit, big = 160.0, 320 * 2**20
    with tempfile.TemporaryDirectory() as d:
        path = Path(d) / "pack"
        with path.open("wb") as f:
            for _ in range(big // 2**20):
                f.write(os.urandom(2**20))
            # Written back before the child maps it: Linux counts a page still dirty in the page
            # cache as Private_Dirty of the mapping process, which is not what the gate measures.
            f.flush()
            os.fsync(f.fileno())
        got = []
        for heap in (8 * 2**20, big):
            p = subprocess.Popen(
                [sys.executable, "-c", CHILD, str(path), str(heap)],
                stdout=subprocess.PIPE,
                text=True,
            )
            try:
                if p.stdout is None or not p.stdout.readline().startswith("ready"):
                    sys.exit(
                        "bench-report self-test failed: memory child did not start"
                    )
                got.append(MEASURE(p.pid) or 0.0)
            finally:
                p.kill()
                p.wait()
    small, large = got
    if not (small <= limit < large):
        sys.exit(
            f"bench-report self-test failed: mapped file and small heap {small:.0f} MB, "
            f"large heap {large:.0f} MB, limit {limit:.0f} MB"
        )


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
    own_load_test()
    page_test()
    commit_test()
    wrap_test()
    footprint_test()
    bl.phase_cpu_test()
    export_test()
    gate_test()
    bc.slowest_page_test(importlib.import_module("bench-run"))
    memory_measure_test()
    print("bench-report self-test: ok")


# Six hex characters with a digit and a letter: a tracker id, not a word like "facade".
TRACKER_ID = re.compile(r"\b(?=[0-9a-f]*[0-9])(?=[0-9a-f]*[a-f])[0-9a-f]{6}\b")


def page_problems(
    text: str, repos: list[str], files: set[str], dates: set[str]
) -> list[str]:
    """What is wrong with generated results pages: a measured repo missing from the table, a
    results date or file named that is not kept, or a tracker id in the text."""
    errs = [
        f"{n} is in the run but not in the table"
        for n in repos
        if f"| {n} |" not in text
    ]
    errs += [
        f"names the results of {d}, which are not kept"
        for d in sorted(set(re.findall(r"\b20\d\d-\d\d-\d\d\b", text)) - dates)
    ]
    errs += [
        f"names {f}, which is not in tests/bench/results"
        for f in sorted(set(re.findall(r"[\w-]+\.json", text)) - files)
    ]
    errs += [
        f"tracker id {m} in the text" for m in sorted(set(TRACKER_ID.findall(text)))
    ]
    return errs


def commit_test() -> None:
    """A run record without its build commit fails, and the doc names the commit it records."""
    machine = {"gitterrace": "gitterrace v0.1.0-1161f1d", "commit": "1161f1d"}
    run = {"when": "2026-11-20T10:00:00", "machine": machine}
    if bd.commit_errors(run) or bd.version(machine) != "gitterrace v0.1.0-1161f1d":
        sys.exit("bench-report self-test failed: a run with its commit was refused")
    for label, m in [
        ("no commit", {"gitterrace": "gitterrace v0.1.0"}),
        (
            "a short commit",
            {"gitterrace": "gitterrace v0.1.0-1161f", "commit": "1161f"},
        ),
        ("a null commit", {"gitterrace": "gitterrace v0.1.0", "commit": None}),
    ]:
        if not bd.commit_errors({**run, "machine": m}):
            sys.exit(f"bench-report self-test failed: {label} passed the commit guard")
    dirty = {"gitterrace": "gitterrace v0.1.0-dirty", "commit": "1161f1d"}
    if bd.version(dirty).count("1161f1d") != 1:
        sys.exit("bench-report self-test failed: the doc version dropped the commit")
    if bd.version({"gitterrace": "gitterrace v0.1.0"}) != "gitterrace v0.1.0":
        sys.exit(
            "bench-report self-test failed: a run with no commit must render plain"
        )
    if bd.commit_errors(
        {**run, "when": "2026-11-03T10:00:00", "machine": {"gitterrace": "x"}}
    ):
        sys.exit("bench-report self-test failed: a run before COMMIT_FROM was refused")


def page_test() -> None:
    """The pages show every measured repo (a withheld one as withheld) and name nothing that is
    not kept, nor any tracker id."""
    raw = published()[-1]
    kept = shown(raw)
    names = [n for n in raw["repos"]]
    files, dates = {f"{run_name(raw)}.json"}, {raw["when"][:10]}
    text = bd.results_readme([kept]) + bd.render()
    if got := page_problems(text, names, files, dates):
        sys.exit(f"bench-report self-test failed: a clean page was flagged: {got}")
    # Built at run time so no id-shaped token sits in the source.
    fake_id = "".join("0a" for _ in range(3))
    for label, tweak, want in [
        (
            "a stale date",
            text + " after the removal of the 2026-01-01 results",
            "2026-01-01",
        ),
        ("a stale file", text + " see 2020-01-01-x.json", "2020-01-01-x.json"),
        ("a tracker id", text + f" see {fake_id} for details", fake_id),
        ("a missing repo", text.replace("| " + names[0] + " |", "| x |"), names[0]),
    ]:
        if not any(want in e for e in page_problems(tweak, names, files, dates)):
            sys.exit(f"bench-report self-test failed: {label} passed the page guard")
    withheld = shown({**raw, "repos": {n: _loud(r) for n, r in raw["repos"].items()}})
    if not withheld["held"] or any(
        n not in bd.results_readme([withheld]) for n in names
    ):
        sys.exit(
            "bench-report self-test failed: a withheld repo vanished from the table"
        )


def _loud(r: Json) -> Json:
    """A copy of a repo record whose sampled load reached its limit."""
    out = json.loads(json.dumps(r))
    out["load"] = {
        "limit": 5.0,
        "phases": {},
        "sampled": {"index": {"max": 9.0, "mean": 2.0, "n": 3}},
    }
    return out


def own_load_test() -> None:
    """The load guard leaves the harness's own process tree out of the measure."""
    rows = [
        ["10", "1", "5"],
        ["11", "10", "150"],
        ["12", "11", "50"],
        ["20", "1", "30"],
    ]
    own, other = bq.split_cores(rows, 10)
    if (round(own, 2), round(other, 2)) != (2.05, 0.3):
        sys.exit(
            f"bench-report self-test failed: own pid tree counted as load {own} {other}"
        )


def wrap_test() -> None:
    """A long list item wraps under 100 columns with a 2-space hang, words unchanged."""
    item = "- **git:** " + " ".join(["word"] * 40)
    got = bd.wrap(item).split("\n")
    if (
        len(got) < 2
        or any(len(x) > bd.WIDTH for x in got)
        or not all(x.startswith("  ") and not x.startswith("   ") for x in got[1:])
        or " ".join(x.strip() for x in got) != item
    ):
        sys.exit(f"bench-report self-test failed: list item wrap {got}")


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


def export_test() -> None:
    """A failed export is shown as failed, never as a time."""
    bad = {"ok": False, "secs": 4029.4, "out_mb": 511.0}
    good = {"ok": True, "secs": 12.3, "out_mb": 92.0}
    capped = {
        **bad,
        "error": "Error: export stopped at the cap of 20000 pages on a page",
    }
    broken = {**bad, "error": "Error: permission denied"}
    got = bd.export_cell({"export": bad})
    if (
        "4029" in got
        or got != bd.FAILED_EXPORT
        or "12.3 s" not in bd.export_cell({"export": good})
        or "(100 commits)" not in bd.export_cell({"export": {**good, "commits": 100}})
        or "commits" in bd.export_cell({"export": {**good, "commits": 500}})
        or "commits" in bd.export_cell({"export": good})
        or bd.export_cell({"export": {**capped, "commits": 100}}) != bd.CAPPED_EXPORT
        or bd.export_cell({"export": capped}) != bd.CAPPED_EXPORT
        or bd.export_cell({"export": broken}) != bd.FAILED_EXPORT
        or "1 run"
        not in bd.export_cell(
            {"cold_samples": 5, "export": {**good, "samples": [12.3]}}
        )
        or "1 run"
        in bd.export_cell({"cold_samples": 5, "export": {**good, "samples": [1, 2]}})
    ):
        sys.exit(f"bench-report self-test failed: export cell {got}")


def footprint_test() -> None:
    """A new run without an idle footprint, or one read before any release, is refused."""

    def run(memory: Json) -> Json:
        return {
            "harness": 2,
            "when": "2026-01-01",
            "machine": {"label": "t"},
            "repos": {"r": {"memory": memory}},
        }

    good = {
        "footprint_idle_mb": 40.0,
        "footprint_start_mb": 9.0,
        "footprint_releases": 1,
    }
    missing = {k: v for k, v in good.items() if k != "footprint_idle_mb"}
    early = {**good, "footprint_releases": 0}
    old = {
        "when": "2025-01-01",
        "machine": {"label": "t"},
        "repos": {"r": {"memory": {}}},
    }
    bad = [m for m in (missing, early) if not footprint(run(m))]
    if footprint(run(good)) or footprint(old) or bad:
        sys.exit(
            f"bench-report self-test failed: footprint guard passed {bad or 'a good run'}"
        )


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
    """The newest run must come from a clean build, timed under its load limit, of a commit
    still in history: landing restamps a commit, so a run recorded on the lane's copy names
    nothing once it lands."""
    errs: list[str] = bd.commit_errors(run)
    m = run["machine"]
    c = m.get("commit")
    if (
        isinstance(c, str)
        and c != bd.PENDING
        and not any(s.startswith(c) for s in own_commits())
    ):
        errs.append(f"{run_name(run)}: machine.commit {c} is not in this history")
    if (m.get("dirty") or "dirty" in m.get("gitterrace", "")) and not run.get("note"):
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


def footprint(run: Json) -> list[str]:
    """A run from harness 2 on records, per repo, the dirty footprint after an idle release and
    of a just-started server. Older runs predate it and are shown as not measured."""
    if run.get("harness", 1) < 2:
        return []
    errs: list[str] = []
    for n, r in run["repos"].items():
        m = r.get("memory", {})
        for key in ("footprint_idle_mb", "footprint_start_mb"):
            if m.get(key) is None:
                errs.append(f"{run_name(run)}: {n} recorded no memory.{key}")
        if m.get("footprint_releases", 0) < 1:
            errs.append(
                f"{run_name(run)}: {n} read its footprint before an idle release"
            )
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


def own_ids() -> list[str]:
    """No result names a commit of gitterrace's own history except the build commit in the
    machine record: the control repo is measured at HEAD and the run date names it. Upstream
    corpus shas stay."""
    own = bl.own_shas(ROOT)
    errs: list[str] = []
    for p in runs():
        rest = {k: v for k, v in json.loads(p.read_text()).items() if k != "machine"}
        if bl.scrub_own(rest, own) != rest:
            errs.append(f"{p.name}: names a commit of this repository")
    return errs


def check(text: str, new: str) -> None:
    want = expected()
    newest = published()[-1]
    errs = pins() + no_inline_svg() + newest_ok(newest) + cold_samples(newest)
    errs += footprint(newest)
    errs += bl.phase_cpu(newest, run_name(newest))
    errs += [
        f"{p.name}: name is not <date>-<machine>.json"
        for p in runs()
        if not NAME.fullmatch(p.name)
    ]
    errs += stray()
    errs += own_ids()
    errs += no_hostname(sorted(actual()) + [DOC])
    for p, body in want.items():
        if not p.exists() or p.read_text() != body:
            errs.append(f"{p.relative_to(ROOT)}: stale or missing")
    errs += [
        f"{p.relative_to(ROOT)}: left over from an older run"
        for p in actual() - set(want)
    ]
    pages = "\n".join(body for p, body in want.items() if p.suffix == ".md") + new
    errs += page_problems(
        pages,
        list(published()[-1]["repos"]),
        {p.name for p in runs()},
        {run["when"][:10] for run in published()},
    )
    if new != text:
        errs.append("docs/benchmarks.md#results is stale")
    if errs:
        sys.exit("\n".join(errs) + "\nrun `make bench-report`")
    print("tests/bench/results and docs/benchmarks.md: up to date")


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
        f"updated {DOC.relative_to(ROOT)} and tests/bench/results from {len(published())} run(s)"
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
