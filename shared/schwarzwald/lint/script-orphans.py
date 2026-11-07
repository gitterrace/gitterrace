#!/usr/bin/env python3
"""CONV-SCRIPT-ORPHANS: every script is reachable from something that is not itself.

A script wired to nothing (no Makefile target, no gate, no doc) is cited as coverage and rots. A
script counts as reached when its file name appears in another tracked file matched by
lint.toml `[scripts] readers`. A truly manual tool says `manual-only` in its first 15 lines and
is exempt. The scripts are the `.sh` and `.py` files under `[scripts] dirs`, minus `exempt`
(hook entry points that the VCS calls by name). An empty scan fails: it reads like a clean one.

  script-orphans.py [--repo DIR]
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-SCRIPT-ORPHANS"
MARKER = "manual-only"


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
    orphans = []
    for n in scripts:
        head = "\n".join((root / n).read_text(encoding="utf-8", errors="replace").splitlines()[:15])
        if MARKER in head:
            continue
        base = Path(n).name
        if not any(base in t for other, t in texts.items() if other != n):
            orphans.append(n)
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
