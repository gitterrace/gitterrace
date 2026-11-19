#!/usr/bin/env python3
"""Re-run the cheap gates over a range of commits handed back without a build.

Work written without cargo needs one command that disagrees when its own diff breaks a
rule. This prints the changed files and the diffstat, picks the cargo-free gates the
touched file types need, runs each once, and reports PASS or FAIL per gate. With --range it
also fails a `test` commit that removes product lines (code above `#[cfg(test)]` in crates/),
and runs the commit-message step of land-check over the range. It does not replace
`scripts/land-check.sh`, which builds and tests; this is the gate before it.

  python3 scripts/lint/commits-check.py --range A..B   the commits to accept
  python3 scripts/lint/commits-check.py                the working tree against HEAD
  python3 scripts/lint/commits-check.py --self-test    prove a failing gate and a product
                                                       removal go red
"""

from __future__ import annotations

import argparse
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[2]
PY = sys.executable
# (gate, command, the suffixes or path prefixes that select it; empty selects on any change)
GATES: list[tuple[str, list[str], tuple[str, ...]]] = [
    ("lines", [PY, "scripts/lint/lines-check.py", "--repo", "."], ()),
    ("names", [PY, "scripts/lint/names-check.py"], ()),
    ("status-words", [PY, "scripts/lint/status-words-check.py"], ()),
    ("stubs", [PY, "scripts/lint/stubs-check.py"], (".rs",)),
    ("tests-modules", [PY, "scripts/lint/tests-check.py"], (".rs",)),
    ("types", [PY, "scripts/lint/types-check.py"], (".rs", "scripts/guards/")),
    ("css-scope", [PY, "scripts/lint/css-scope-check.py"], (".css",)),
    ("css-button", [PY, "scripts/lint/css-button-check.py"], (".css", ".html", ".js")),
    (
        "theme",
        [PY, "scripts/release/site/theme-gen.py", "--check"],
        (".css", "scripts/release/site/theme-", "vendor/schwarzwald/kit/"),
    ),
    (
        "make-targets",
        [PY, "vendor/schwarzwald/kit/lint/make-targets.py", "--repo", "."],
        (".md", "Makefile"),
    ),
    (
        "actions-pin",
        [PY, "vendor/schwarzwald/kit/lint/actions-pin.py", "--repo", "."],
        (".github/",),
    ),
]


def run(cmd: list[str]) -> tuple[int, str]:
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=False)
    return p.returncode, (p.stdout + p.stderr).strip()


def changed(rng: str | None) -> list[str]:
    _, out = run(["git", "diff", "--name-only", rng or "HEAD"])
    return [f for f in out.splitlines() if f]


def selected(files: list[str]) -> list[tuple[str, list[str]]]:
    def hits(keys: tuple[str, ...]) -> bool:
        return not keys or any(
            f.endswith(k) or f.startswith(k) for f in files for k in keys
        )

    return [(name, cmd) for name, cmd, keys in GATES if hits(keys)]


def is_test_path(path: str) -> bool:
    name = PurePosixPath(path).name
    return (
        "/tests/" in f"/{path}"
        or name.endswith(("tests.rs", "_tests.rs"))
        or path.startswith("crates/gitterrace-testkit/")
    )


def prod_removals(path: str, removed: list[int], old: str) -> list[int]:
    """Non-blank old-side lines a test commit removed from product code in `path`."""
    if is_test_path(path) or not path.startswith("crates/"):
        return []
    lines = old.splitlines()
    removed = [n for n in removed if 0 < n <= len(lines) and lines[n - 1].strip()]
    if not path.endswith(".rs"):
        return removed
    cut = next(
        (i + 1 for i, s in enumerate(lines) if s.strip() == "#[cfg(test)]"), None
    )
    return removed if cut is None else [n for n in removed if n < cut]


def removed_lines(diff: str) -> dict[str, list[int]]:
    """Old-side removed line numbers per file, from a `git show -U0` diff."""
    out: dict[str, list[int]] = {}
    path = None
    for line in diff.splitlines():
        if line.startswith("--- "):
            path = line[6:] if line.startswith("--- a/") else None
        elif line.startswith("@@ ") and path:
            start, _, count = line.split()[1].lstrip("-").partition(",")
            n = int(count) if count else 1
            out.setdefault(path, []).extend(range(int(start), int(start) + n))
    return out


def test_commits_removing_product(rng: str) -> list[str]:
    _, shas = run(["git", "rev-list", "--no-merges", rng])
    hits = []
    for sha in shas.split():
        _, subject = run(["git", "log", "-1", "--format=%s", sha])
        if not subject.startswith(("test(", "test:")):
            continue
        _, diff = run(["git", "show", "-U0", "--format=", sha])
        for path, removed in removed_lines(diff).items():
            code, old = run(["git", "show", f"{sha}~:{path}"])
            bad = prod_removals(path, removed, old) if code == 0 else []
            if bad:
                hits.append(
                    f"{sha[:9]} {path}: {len(bad)} product line(s) from line {bad[0]}"
                )
    return hits


def self_test() -> int:
    ok = run([PY, "-c", "raise SystemExit(3)"])[0] == 3
    rust = "fn a() {}\nfn b() {}\n\n#[cfg(test)]\nmod tests {\n    fn t() {}\n}\n"
    ok = ok and prod_removals("crates/x/src/a.rs", [1, 2, 3, 6], rust) == [1, 2]
    ok = ok and not prod_removals("crates/x/src/tests.rs", [1], rust)
    ok = ok and not prod_removals("scripts/x.py", [1], "x\n")
    diff = "--- a/crates/x/src/a.rs\n+++ b/crates/x/src/a.rs\n@@ -2,2 +1,0 @@\n-x\n-y\n"
    ok = ok and removed_lines(diff) == {"crates/x/src/a.rs": [2, 3]}
    ok = ok and [n for n, _ in selected(["crates/a/src/x.css"])].count("css-scope") == 1
    ok = ok and "css-scope" not in [n for n, _ in selected(["crates/a/src/x.rs"])]
    print("commits-check self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    ap.add_argument(
        "--range", help="A..B, the commits to accept; default: the working tree"
    )
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    files = changed(a.range)
    if not files:
        print("commits-check: nothing changed")
        return 0
    print("files:\n  " + "\n  ".join(files))
    print("diffstat:\n" + run(["git", "diff", "--stat", a.range or "HEAD"])[1])
    gates = selected(files)
    print("gates: " + ", ".join(n for n, _ in gates))
    with ThreadPoolExecutor() as pool:
        results = list(pool.map(lambda g: (g[0], *run(g[1])), gates))
    fails = 0
    for name, code, out in results:
        if code:
            fails += 1
            print(out)
        print(f"{'FAIL' if code else 'PASS'} {name}")
    if a.range:
        hits = test_commits_removing_product(a.range)
        for h in hits:
            print(h)
        print(f"{'FAIL' if hits else 'PASS'} test-commits-keep-product")
        fails += bool(hits)
        base, _, tip = a.range.partition("..")
        # land-check reads messages over BASE..HEAD, so it speaks for the range only at HEAD.
        if (
            run(["git", "rev-parse", tip or "HEAD"])[1]
            == run(["git", "rev-parse", "HEAD"])[1]
        ):
            code, out = run(["bash", "scripts/land-check.sh", "--messages", base])
            if code:
                print(out)
            print(f"{'FAIL' if code else 'PASS'} messages")
            fails += bool(code)
        else:
            print(f"SKIP messages: check out {tip} to run them")
    print(
        "Judgement is not checked here: read the diff for comments that earn their place."
    )
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
