"""The release regression gate: the newest run against the newest run of the previous release.

Owner rule: a release goes out only if the numbers get better or do not get worse. Every
metric here is lower-is-better. A change counts as a regression only when it is worse than the
noise band of that metric (NOISE and band below), never when it merely looks bigger. There is
no override: a refused release is fixed by a faster build or a run on the right machine, and
the band is never widened to let a run pass.

Stdlib only. bench-report.py runs it as `--regress [--against <run>]` and from its self-test.
"""

from __future__ import annotations

import importlib
import re
import sys
from typing import Any

bc = importlib.import_module("bench-chart-lib")

Json = dict[str, Any]

# metric class -> (relative floor, absolute floor). A change inside max(relative * old,
# absolute) is noise whatever the samples say, and the spread of the two runs' own samples
# (min..max) widens it further. Why each value:
#   secs      5% and 0.05 s: a timed build of the same code moves a few percent between runs on
#             a shared laptop, and under 50 ms the clock and the scheduler dominate.
#   ms (page) 5% and 0.5 ms: the same few percent. Under 5 ms a page is dominated by one
#             syscall or one wakeup, so it gets the wider NOISE_SMALL_MS_REL below.
#   mb        5% and 1 MB: the allocator rounds to pages and the footprint reader to 1 MB.
#   pct       10% and 0.1 points: idle CPU is a few hundredths of a core, sampled coarsely.
NOISE: dict[str, tuple[float, float]] = {
    "secs": (0.05, 0.05),
    "ms": (0.05, 0.5),
    "mb": (0.05, 1.0),
    "pct": (0.10, 0.1),
}
NOISE_SMALL_MS = 5.0
NOISE_SMALL_MS_REL = 0.10
CLASS = {
    "index_secs": "secs",
    "ready_secs": "secs",
    "export_secs": "secs",
    "page_cold_ms": "ms",
    "page_warm_ms": "ms",
    "rss_mb": "mb",
    "footprint_idle_mb": "mb",
    "idle_cpu_pct": "pct",
}
UNIT = {"secs": "s", "ms": "ms", "mb": "MB", "pct": "%"}

VERSION = re.compile(r"v?(\d+\.\d+\.\d+)")


def release_of(run: Json) -> str:
    """The release a run was built for: x.y.z without the commit or -dirty suffix."""
    m = VERSION.search(run.get("machine", {}).get("gitterrace", ""))
    return m.group(1) if m else ""


def label(run: Json) -> str:
    return str(run.get("machine", {}).get("label", ""))


def band(cls: str, old: float, spread: float) -> float:
    """How much worse a value may get before it counts: the larger of the floor and the
    samples' own spread."""
    rel, floor = NOISE[cls]
    if cls == "ms" and old < NOISE_SMALL_MS:
        rel = NOISE_SMALL_MS_REL
    return max(rel * old, floor, spread)


def sample(entry: Json | None) -> tuple[float, float] | None:
    """(value, spread) of a budget entry or a page timing, None when it was not measured."""
    if not entry or entry.get("value") is None:
        return None
    lo, hi = entry.get("min"), entry.get("max")
    spread = float(hi) - float(lo) if lo is not None and hi is not None else 0.0
    return float(entry["value"]), spread


def page_samples(page: Json, kind: str) -> tuple[float, float] | None:
    if kind == "cold":
        c = page.get("cold") or {}
        s = c.get("samples") or []
        v = c.get("total_ms")
        return None if v is None else (float(v), float(max(s) - min(s)) if s else 0.0)
    w = page.get("warm_total_ms") or {}
    v = w.get("median")
    lo, hi = w.get("min"), w.get("max")
    if v is None:
        return None
    return float(v), float(hi - lo) if lo is not None and hi is not None else 0.0


def measures(run: Json, name: str) -> dict[str, tuple[str, tuple[float, float] | None]]:
    """Every compared measure of one repo: label -> (class, (value, spread) or None)."""
    r = run["repos"].get(name) or {}
    out: dict[str, tuple[str, tuple[float, float] | None]] = {}
    for key in bc.METRICS:
        out[key] = (CLASS[key], sample(r.get("budget", {}).get(key)))
    for p, page in sorted((r.get("pages") or {}).items()):
        if page.get("status", 200) != 200:
            continue
        for kind in ("cold", "warm"):
            out[f"page {p} {kind}"] = ("ms", page_samples(page, kind))
    return out


def compare(old: Json, new: Json) -> list[Json]:
    """One row per repo and measure: repo, metric, old, new, delta, allowed, verdict. The
    verdict is regressed, improved, same (inside the band) or unmeasured. A measure the new run
    lacks but the old one has is unmeasured and refuses (the harness lost a check), one only
    the old run lacks is unmeasured and does not (an older harness)."""
    rows: list[Json] = []
    for name in new["repos"]:
        if name not in old["repos"]:
            continue
        a, b = measures(old, name), measures(new, name)
        for m in sorted(set(a) | set(b), key=lambda k: (k not in bc.METRICS, k)):
            cls = (a.get(m) or b[m])[0]
            o, n = (a.get(m) or (cls, None))[1], (b.get(m) or (cls, None))[1]
            row: Json = {"repo": name, "metric": m, "class": cls, "old": None, "new": None}
            if o is None or n is None:
                row["verdict"] = "unmeasured"
                row["refuse"] = o is not None and n is None
                row["old"], row["new"] = (o and o[0]), (n and n[0])
                if m == "rss_mb" and o and n:
                    row["refuse"] = False
                rows.append(row)
                continue
            if m == "rss_mb" and not bc.comparable(m, old["repos"][name], new["repos"][name]):
                row.update(verdict="unmeasured", refuse=False, old=o[0], new=n[0])
                rows.append(row)
                continue
            allowed = band(cls, o[0], max(o[1], n[1]))
            delta = n[0] - o[0]
            verdict = "regressed" if delta > allowed else "improved" if -delta > allowed else "same"
            row.update(old=o[0], new=n[0], delta=delta, allowed=allowed, verdict=verdict)
            row["refuse"] = verdict == "regressed"
            rows.append(row)
    for name in old["repos"]:
        if name not in new["repos"]:
            gone = {"repo": name, "metric": "(whole repo)", "class": "secs", "old": None}
            rows.append({**gone, "new": None, "verdict": "unmeasured", "refuse": True})
    return rows


def baseline(runs: list[Json], against: str | None) -> tuple[Json | None, str]:
    """The run to compare the newest against, and a note when there is none. By default the
    newest earlier run of another release, so a pre-release run never moves the bar."""
    newest, earlier = runs[-1], runs[:-1]
    if against:
        for r in earlier:
            if against in (bc.run_name(r), r["when"][:10], f"{bc.run_name(r)}.json"):
                return r, ""
        return None, f"no kept run named {against}"
    rel = release_of(newest)
    for r in reversed(earlier):
        if release_of(r) and release_of(r) != rel:
            return r, ""
    return None, (
        f"no kept run of a release before {rel or 'this one'}: record one on this machine, "
        "or the release has nothing to be compared with"
    )


def regress(runs: list[Json], against: str | None = None) -> tuple[list[Json], list[str], bool]:
    """(rows, notes, refused) for the newest run of runs, which are sorted oldest first."""
    if not runs:
        return [], ["no benchmark run is kept"], True
    old, note = baseline(runs, against) if len(runs) > 1 else (None, "only one run is kept")
    if old is None:
        return [], [f"NO COMPARISON: {note}"], True
    new = runs[-1]
    head = f"newest {bc.run_name(new)} ({release_of(new) or '?'}) against {bc.run_name(old)}"
    if label(old) != label(new) or not label(new):
        return (
            [],
            [
                f"NO COMPARISON: {head} are on different machines "
                f"('{label(new)}' and '{label(old)}'): numbers from two machines prove nothing",
            ],
            True,
        )
    rows = compare(old, new)
    notes = [head]
    return rows, notes, any(r["refuse"] for r in rows)


def fmt(cls: str, v: float | None) -> str:
    return "-" if v is None else f"{v:.2f} {UNIT[cls]}"


def table(rows: list[Json]) -> list[str]:
    """Regressions, improvements and unmeasured rows as a table, then a count of the rest."""
    shown = [r for r in rows if r["verdict"] != "same"]
    out: list[str] = []
    if shown:
        out += ["| repo | metric | old | new | delta | verdict |", "|---|---|---|---|---|---|"]
    for r in sorted(shown, key=lambda r: (r["verdict"] != "regressed", r["repo"], r["metric"])):
        d = "-"
        if r.get("delta") is not None:
            pct = f" ({r['delta'] / r['old'] * 100:+.0f}%)" if r["old"] else ""
            d = f"{r['delta']:+.2f} {UNIT[r['class']]}{pct}, band {r['allowed']:.2f}"
        v = r["verdict"] + (" REFUSED" if r["refuse"] else "")
        out.append(
            f"| {r['repo']} | {r['metric']} | {fmt(r['class'], r['old'])} | "
            f"{fmt(r['class'], r['new'])} | {d} | {v} |"
        )
    out.append(f"{sum(r['verdict'] == 'same' for r in rows)} measure(s) inside the noise band")
    return out


def run_cli(against: str | None) -> int:
    runs = [bc.load(p) for p in bc.runs()]
    rows, notes, refused = regress(runs, against)
    print("\n".join(notes + table(rows)))
    bad = sum(r["refuse"] for r in rows)
    tail = f", {bad} measure(s)" if bad else ""
    print(f"bench regression gate: {'REFUSED' if refused else 'ok'}{tail}")
    return 1 if refused else 0


# Self-test ----------------------------------------------------------------------------------


def synth(when: str, version: str, machine: str = "m1", scale: float = 1.0, **kw: Any) -> Json:
    """A run of one repo: index 10 s with samples 9.9..10.1, a 2 ms page, 100 MB peak."""
    spread = kw.get("spread", 0.2)
    idx = 10.0 * scale
    budget: Json = {
        "index_secs": {"value": idx, "min": idx - spread / 2, "max": idx + spread / 2},
        "rss_mb": {"value": 100.0 * scale},
        "footprint_idle_mb": {"value": 20.0 * scale},
    }
    for k in kw.get("drop", ()):
        del budget[k]
    page = {
        "status": 200,
        "cold": {"total_ms": 2.0 * scale, "samples": [2.0 * scale, 2.1 * scale]},
        "warm_total_ms": {"median": 1.0 * scale, "min": 0.9 * scale, "max": 1.1 * scale},
    }
    return {
        "when": when,
        "machine": {"label": machine, "gitterrace": f"gitterrace v{version}-abcdef0"},
        "repos": {"r": {"index": {}, "budget": budget, "pages": {"p": page}}},
    }


def self_test() -> None:
    def fail(msg: str) -> None:
        sys.exit(f"bench-regress self-test failed: {msg}")

    base = synth("2026-01-01", "0.1.0")
    pre = synth("2026-01-05", "0.2.0", scale=1.5)

    def go(new: Json, *before: Json, against: str | None = None):
        return regress([*before, new], against)

    def verdicts(rows: list[Json]) -> dict[tuple[str, str], str]:
        return {(r["repo"], r["metric"]): r["verdict"] for r in rows}

    rows, _, refused = go(synth("2026-01-10", "0.2.0", scale=1.2), base)
    if not refused or verdicts(rows)[("r", "index_secs")] != "regressed":
        fail("a 20% slower run was not refused")
    if "index_secs" not in "\n".join(table(rows)) or "REFUSED" not in "\n".join(table(rows)):
        fail("the refusal table does not name the metric")
    rows, _, refused = go(synth("2026-01-10", "0.2.0", scale=1.03), base)
    if refused or verdicts(rows)[("r", "index_secs")] != "same":
        fail(f"a 3% wobble inside the band was refused: {table(rows)}")
    rows, _, refused = go(synth("2026-01-10", "0.2.0", scale=0.8), base)
    if refused or verdicts(rows)[("r", "index_secs")] != "improved":
        fail("a faster run was refused or its improvement not reported")
    if "improved" not in "\n".join(table(rows)):
        fail("an improvement is not in the table")
    _, notes, refused = go(synth("2026-01-10", "0.2.0", machine="m2"), base)
    if not refused or "different machines" not in "".join(notes):
        fail("a run on another machine was compared or passed")
    older = synth("2026-01-01", "0.1.0", drop=["rss_mb"])
    rows, _, refused = go(synth("2026-01-10", "0.2.0"), older)
    if refused or verdicts(rows)[("r", "rss_mb")] != "unmeasured":
        fail("a metric the older harness lacked was not reported as unmeasured")
    rows, _, refused = go(synth("2026-01-10", "0.2.0", drop=["rss_mb"]), base)
    if not refused or verdicts(rows)[("r", "rss_mb")] != "unmeasured":
        fail("a metric the new run lost did not refuse")
    # The pre-release run is slow, the newest is only 10% faster than it: still 15% over the
    # previous release, and refused, because a pre-release run never moves the bar.
    rows, _, refused = go(synth("2026-01-10", "0.2.0", scale=1.35), base, pre)
    if not refused:
        fail("a pre-release run moved the baseline")
    _, notes, refused = go(synth("2026-01-10", "0.2.0"), pre)
    if not refused or "NO COMPARISON" not in "".join(notes):
        fail("a release with no earlier release run passed")
    # The samples' own spread widens the band: 20% slower with a 4 s spread is noise.
    rows, _, _ = go(synth("2026-01-10", "0.2.0", scale=1.2, spread=4.0), base)
    if verdicts(rows)[("r", "index_secs")] != "same":
        fail("a spread wider than the change was refused")
    # Tiny values: 0.3 ms on a 1 ms page is under the absolute floor.
    _, _, refused = regress([base, synth("2026-01-10", "0.2.0", scale=1.0)])
    if refused:
        fail("an identical run was refused")
    _, _, refused = go(synth("2026-01-10", "0.2.0", scale=1.2), base, against="2026-01-01")
    if not refused:
        fail("--against the same baseline did not refuse")
    if go(synth("2026-01-10", "0.2.0"), base, against="nope")[2] is not True:
        fail("--against an unknown run passed")
    for cls in NOISE:
        if band(cls, 0.0, 0.0) <= 0:
            fail(f"{cls} has no floor")
