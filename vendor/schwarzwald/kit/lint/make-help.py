#!/usr/bin/env python3
"""CONV-MAKE-HELP: a product's Makefile reads the same way as every other product's.

1. It includes the kit's `configs/help.mk`, so `make help` prints one shared layout.
2. Every `## target: text` line names a target the Makefile defines, and sits under a
   `##@ Section` line, so the list is grouped.
3. Every target of `[make] core` (lint.toml) is defined and has a `##` line. The core list is
   the vocabulary a person types in any product: the same name does the same job everywhere.
4. A documented target names one job: lowercase kebab-case, no stale word (old, new, tmp, v2).

The Makefiles read are the files in lint.toml `[scope] make`.

  make-help.py [--repo DIR]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-MAKE-HELP"
TARGET = re.compile(r"^([A-Za-z0-9_.-]+(?:\s+[A-Za-z0-9_.-]+)*)\s*:(?![=:])", re.M)
DOC = re.compile(r"^## ([A-Za-z0-9_.-]+): \S", re.M)
INCLUDE = re.compile(r"^-?include\s+\S*configs/help\.mk\s*$", re.M)
NAME = re.compile(r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$")
# A variable the shared help.mk reads (`make help TARGET=x`). A product that sets it for itself
# (a cargo target dir, say) makes `make help` explain that value instead of listing every target.
HELP_VARS = ("TARGET",)
ASSIGN = re.compile(r"^\s*(?:override\s+|export\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::{1,3}|\?|\+|!)?=", re.M)
STALE = {"old", "new", "tmp", "temp", "copy", "final", "v2"}


def problems(rel: str, text: str, core: list[str]) -> list[str]:
    defined = {t for m in TARGET.finditer(text) for t in m.group(1).split()}
    out = []
    if not INCLUDE.search(text):
        out.append(f"{rel}: does not include the kit's configs/help.mk")
    for m in ASSIGN.finditer(text):
        if m.group(1) in HELP_VARS:
            n = text.count("\n", 0, m.start()) + 1
            out.append(f"{rel}:{n}: sets {m.group(1)}, which help.mk reads for `make help "
                       f"{m.group(1)}=x`; rename it")
    section = False
    documented = set()
    for n, line in enumerate(text.splitlines(), 1):
        if line.startswith("##@ "):
            section = True
            continue
        m = DOC.match(line)
        if not m:
            continue
        t = m.group(1)
        documented.add(t)
        if t != "help" and t not in defined:
            out.append(f"{rel}:{n}: ## {t} documents a target the Makefile does not define")
        if not section:
            out.append(f"{rel}:{n}: ## {t} sits above every ##@ section")
        if not NAME.match(t) or STALE & set(t.split("-")):
            out.append(f"{rel}:{n}: {t} is not one lowercase kebab name for one job")
    for t in core:
        if t == "help":
            continue
        if t not in defined:
            out.append(f"{rel}: the core target {t} is missing")
        elif t not in documented:
            out.append(f"{rel}: the core target {t} has no ## line")
    return out


def self_test() -> bool:
    good = "##@ Build\n## build: Release binary\nbuild:\n\ttrue\nfmt-check:\n\ttrue\n"
    good += "include schwarzwald/kit/configs/help.mk\n"
    cases = [
        (good, ["build"], 0),
        (good.replace("include", "# include"), ["build"], 1),
        (good + "## gone: Nothing\n", ["build"], 1),
        ("## build: Above\n" + good.replace("## build: Release binary\n", ""), ["build"], 1),
        (good, ["build", "test"], 1),
        (good.replace("## build: Release binary\n", ""), ["build"], 1),
        (good.replace("build", "build-old"), [], 1),
        ("TARGET ?= target\n" + good, ["build"], 1),
        ("IOS_DEPLOYMENT_TARGET = 26.0\n" + good, ["build"], 0),
    ]
    ok = all((len(problems("M", t, c)) > 0) == bool(bad) for t, c, bad in cases)
    return config.self_check(RULE, ok, "a planted defect must fail and a clean Makefile pass")


def main(argv: list[str]) -> int:
    root, _ = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    names = config.tracked(root, cfg["scope"]["exclude"])
    makes = [n for n in names if config.matches(n, cfg["scope"]["make"])]
    if not makes:
        print(f"PASS [{RULE}] no Makefile in [scope] make, nothing to check")
        return 0
    core = cfg["make"]["core"]
    bad = []
    for rel in makes:
        bad += problems(rel, (root / rel).read_text(encoding="utf-8", errors="replace"), core)
    return config.report(RULE, bad, f"{len(bad)} problem(s) in the Makefile's help",
                         f"{len(makes)} Makefile(s) print the shared help")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
