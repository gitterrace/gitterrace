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

import importlib
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
    wrap_test()
    print("bench-report self-test: ok")


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


def unknown_ids() -> list[str]:
    """Every gitterrace commit a result names must exist in this history. A shallow clone
    cannot tell, so it is skipped there."""
    shallow = subprocess.run(
        ["git", "rev-parse", "--is-shallow-repository"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if shallow != "false":
        return []
    errs: list[str] = []
    for p in runs():
        run = load(p)
        control = run["repos"].get("control", {})
        ids = {run["machine"].get("gitterrace_sha"), control.get("sha")}
        ids.add(control.get("probe", {}).get("commit"))
        for i in sorted(x for x in ids if x):
            ok = subprocess.run(
                ["git", "cat-file", "-e", f"{i}^{{commit}}"],
                cwd=ROOT,
                capture_output=True,
            )
            if ok.returncode:
                errs.append(f"{p.name}: commit {i[:10]} is not in this history")
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
    errs += unknown_ids()
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
