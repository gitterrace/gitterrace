#!/usr/bin/env python3
"""CONV-SCRIPT-ORPHANS: every script is reachable from something that is not itself.

A script wired to nothing (no Makefile target, no gate, no doc) is cited as coverage and rots. A
script counts as reached when its file name appears in another tracked file matched by
lint.toml `[scripts] readers`, or when a reached python script imports it (`import x`,
`from x import`, `importlib.import_module("x")` by literal name, matched on the file stem, so a
`*-lib.py` helper follows its caller). A truly manual tool says `manual-only` in its first 15 lines and
is exempt. The scripts are the `.sh` and `.py` files under `[scripts] dirs`, minus `exempt`
(hook entry points that the VCS calls by name). An empty scan fails: it reads like a clean one.

  script-orphans.py [--repo DIR]
"""

from __future__ import annotations

import ast
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-SCRIPT-ORPHANS"
MARKER = "manual-only"


def imported(text: str) -> set[str]:
    """Module names a python source imports by literal name, every dotted part included."""
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return set()
    out: set[str] = set()
    for node in ast.walk(tree):
        names: list[str] = []
        if isinstance(node, ast.Import):
            names = [a.name for a in node.names]
        elif isinstance(node, ast.ImportFrom) and node.module:
            names = [node.module] + [f"{node.module}.{a.name}" for a in node.names]
        elif (isinstance(node, ast.Call) and node.args and isinstance(node.args[0], ast.Constant)
              and isinstance(node.args[0].value, str)
              and getattr(node.func, "attr", getattr(node.func, "id", "")) in ("import_module", "__import__")):
            names = [node.args[0].value]
        for n in names:
            out.update(n.split("."))
    return out


def import_reached(scripts: list[str], reached: set[str], root: Path) -> set[str]:
    """Grow reached with every script a reached python script imports, until nothing is added."""
    by_stem: dict[str, list[str]] = {}
    for n in scripts:
        if n.endswith(".py"):
            by_stem.setdefault(Path(n).stem, []).append(n)
    todo = [n for n in reached if n.endswith(".py")]
    while todo:
        text = (root / todo.pop()).read_text(encoding="utf-8", errors="replace")
        for mod in imported(text):
            for n in by_stem.get(mod, []):
                if n not in reached:
                    reached.add(n)
                    todo.append(n)
    return reached


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    s = cfg["scripts"]
    names = config.tracked(root, cfg["scope"]["exclude"])
    scripts = [n for n in names if n.endswith((".sh", ".py"))
               and any(n.startswith(d.rstrip("/") + "/") for d in s["dirs"]) and not config.matches(n, s["exempt"])]
    if not scripts:
        print(f"FAIL [{RULE}] no scripts found under {s['dirs']}: this check inspected nothing")
        return 1
    texts = {}
    for n in names:
        if config.matches(n, s["readers"]):
            try:
                texts[n] = (root / n).read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                pass
    reached = set()
    for n in scripts:
        head = "\n".join((root / n).read_text(encoding="utf-8", errors="replace").splitlines()[:15])
        base = Path(n).name
        if MARKER in head or any(base in t for other, t in texts.items() if other != n):
            reached.add(n)
    reached = import_reached(scripts, reached, root)
    orphans = [n for n in scripts if n not in reached]
    if orphans:
        print(f"FAIL [{RULE}] {len(orphans)} script(s) nothing calls")
        for o in orphans:
            print(f"  {o}")
        print(f"  Wire it into the Makefile, a gate or a doc, delete it, or write {MARKER} in its header.")
        return 1
    print(f"PASS [{RULE}] all {len(scripts)} scripts are reachable")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
