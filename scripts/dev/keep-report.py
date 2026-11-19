#!/usr/bin/env python3
"""List every item in crates/*/src with its prod and test callers.

Output TSV: item, file:line, prod_callers, test_callers, lines.
Callers are word matches of the item name outside its own definition line,
over Rust sources and askama templates. Test code is a file under tests/,
a tests.rs or tests/ module file, or the tail of a file after `#[cfg(test)]`.
Methods carry their impl type as `Type::name`; matching is by name only, so
a shared method name overcounts (never undercounts) callers.

Paths and globs come from DEFAULTS, overridable with --config FILE.json.\nStdlib only. Run with --self-test to check the parser on a small fixture.
"""

from __future__ import annotations

import os
import re
from collections import defaultdict
from pathlib import Path

ITEM = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+fn|async\s+fn|unsafe\s+fn|fn|struct|enum|const|static|trait|type)\s+([A-Za-z_][A-Za-z0-9_]*)"
)
IMPL = re.compile(
    r"^\s*impl(?:<[^>]*>)?\s+(?:[A-Za-z0-9_:<>, ]+\s+for\s+)?([A-Za-z_][A-Za-z0-9_]*)"
)
WORD = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")


DEFAULTS = {
    # directories whose sources define the items to inventory
    "item_globs": ["crates/*/src"],
    # directories scanned for callers (items' own crates plus tools like xtask)
    "caller_roots": ["crates", "xtask"],
    # file suffixes scanned for callers; templates call methods too
    "caller_exts": [".rs", ".html"],
    # regexes over a relative path that mark a whole file as test code
    "test_paths": [
        r"(^|/)tests(/|$)",
        r"(^|/)tests?(_[^/]*)?\.rs$",
        r"_tests\.rs$",
        r"(^|/)testing\.rs$",
        r"-testkit/",
    ],
    "skip_dirs": ["target", "node_modules", ".git"],
}
CONF = dict(DEFAULTS)


def is_test_path(path: str) -> bool:
    path = path.replace(os.sep, "/")
    return any(re.search(p, path) for p in CONF["test_paths"])


def split_lines(path: str, text: str):
    """Yield (lineno, line, is_test)."""
    whole = is_test_path(path)
    pending, depth, until = False, 0, None
    for i, line in enumerate(text.splitlines(), 1):
        s = line.strip()
        if "#[cfg(test)]" in s or "#[cfg(any(test" in s or "#[cfg(all(test" in s:
            pending = True
        elif pending and not s.startswith("#"):
            # the attributed item: a block runs to its closing brace,
            # a `mod x;` or `use` line is test-only on its own
            pending = False
            if "{" in s and until is None:
                until = depth
            elif until is None:
                yield i, line, True
                depth += line.count("{") - line.count("}")
                continue
        in_test = whole or until is not None or pending
        depth += line.count("{") - line.count("}")
        if until is not None and depth <= until:
            in_test, until = True, None
        yield i, line, in_test


def item_spans(lines: list[tuple[int, str, bool]]):
    """Return [(name, qual, lineno, nlines)] for items, sized by brace depth."""
    out = []
    impl_type: str | None = None
    impl_depth, depth = 0, 0
    open_items = []
    for i, line, test in lines:
        if test:
            depth += line.count("{") - line.count("}")
            continue
        m = IMPL.match(line)
        if m and impl_type is None:
            impl_type, impl_depth = m.group(1), depth
        m = ITEM.match(line)
        if m and not line.lstrip().startswith("//"):
            name = m.group(1)
            qual = f"{impl_type}::{name}" if impl_type and depth > impl_depth else name
            rec = [name, qual, i, 1]
            out.append(rec)
            if "{" in line or not line.rstrip().endswith(";"):
                open_items.append((rec, depth))
        depth += line.count("{") - line.count("}")
        for rec, d in list(open_items):
            if depth <= d and ("}" in line or line.rstrip().endswith(";")):
                rec[3] = i - rec[2] + 1
                open_items.remove((rec, d))
        if impl_type is not None and depth <= impl_depth and "}" in line:
            impl_type = None
    return [tuple(r) for r in out]


def item_dirs(root: str):
    import glob

    return [
        os.path.relpath(d, root)
        for g in CONF["item_globs"]
        for d in glob.glob(os.path.join(root, g))
    ]


def scan(root: str):
    files = []
    for top in CONF["caller_roots"]:
        for base, dirs, names in os.walk(os.path.join(root, top)):
            dirs[:] = [d for d in dirs if d not in CONF["skip_dirs"]]
            files += [
                os.path.join(base, n) for n in names if n.endswith(tuple(CONF["caller_exts"]))
            ]
    srcs = item_dirs(root)
    prod, test = defaultdict(int), defaultdict(int)
    items = []
    for f in sorted(files):
        text = Path(f).read_text(encoding="utf-8", errors="replace")
        rel = os.path.relpath(f, root)
        lines = list(split_lines(rel, text))
        if (
            f.endswith(".rs")
            and any(rel.startswith(d + os.sep) for d in srcs)
            and not is_test_path(rel)
        ):
            for name, qual, ln, n in item_spans(lines):
                items.append((name, qual, rel, ln, n))
        for _, line, t in lines:
            if line.lstrip().startswith("//") and not line.lstrip().startswith("///"):
                continue
            for w in WORD.findall(line):
                (test if t else prod)[w] += 1
    return items, prod, test


def rows(root: str):
    items, prod, test = scan(root)
    defs = defaultdict(int)
    for name, *_ in items:
        defs[name] += 1
    for name, qual, rel, ln, n in items:
        # subtract each definition site of this name from prod counts
        yield qual, f"{rel}:{ln}", max(prod[name] - defs[name], 0), test[name], n


def self_test():
    import tempfile

    d = tempfile.mkdtemp()
    src = os.path.join(d, "crates", "a", "src")
    os.makedirs(src)
    Path(src, "lib.rs").write_text(
        "pub fn used() {}\nfn once() {\n    used();\n}\nfn dead() {}\n"
        "struct S;\nimpl S {\n    fn m(&self) {}\n}\nfn main() { once(); S.m(); }\n"
        "#[cfg(test)]\nmod tests {\n    fn t() { super::dead(); }\n}\n"
        "#[cfg(test)]\nmod more;\nfn after() {}\nfn x() { after(); }\n"
    )
    got = {r[0]: r for r in rows(d)}
    assert got["used"][2:4] == (1, 0), got["used"]
    assert got["once"][2:] == (1, 0, 3), got["once"]
    assert got["dead"][2:4] == (0, 1), got["dead"]
    assert got["S::m"][2] == 1, got["S::m"]
    assert "t" not in got
    assert got["after"][2] == 1, got["after"]
    print("self-test ok")


def main():
    import argparse
    import json

    ap = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    ap.add_argument("root", nargs="?", default=".")
    ap.add_argument("--config", help="JSON file overriding keys of DEFAULTS")
    ap.add_argument("--out", default="keep-inventory.tsv")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if a.config:
        CONF.update(json.loads(Path(a.config).read_text()))
    root, out = a.root, os.path.expanduser(a.out)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as fh:
        fh.write("item\tfile:line\tprod_callers\ttest_callers\tlines\n")
        for r in rows(root):
            fh.write("\t".join(map(str, r)) + "\n")
    print(out)


if __name__ == "__main__":
    main()
