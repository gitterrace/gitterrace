#!/usr/bin/env python3
"""Fail on an ignored Rust test whose body only panics.

A test marked `#[ignore]` whose body is nothing but `panic!`, `todo!` or `unimplemented!`
(with or without a message) checks nothing, yet its name reads like a guard in the test list.
Such a stub must be written for real or deleted. Comments inside the body do not count as
content.

  python3 scripts/lint/stubs-check.py [root]      scan crates/ and xtask/
  python3 scripts/lint/stubs-check.py --self-test prove it goes red on planted stubs, green
                                                  on real tests
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DIRS = ("crates", "xtask")

# Attributes, then a test fn header. The body is matched by brace counting below.
HEAD = re.compile(
    r"((?:[ \t]*#\[[^\n]*\][ \t]*\n|[ \t]*//[^\n]*\n)+)"
    r"[ \t]*(?:pub(?:\([a-z]+\))? )?(?:async )?fn (\w+)\s*\([^)]*\)[^{;]*\{"
)
STUB = re.compile(
    r"^(?:panic|todo|unimplemented)!\s*(?:\((?:[^()]|\([^()]*\))*\)|\[[^\]]*\]|\{[^}]*\})\s*;?$"
)


def body_at(text: str, start: int) -> str:
    """The text between the `{` before `start` and its matching `}`."""
    depth, i = 1, start
    while i < len(text) and depth:
        c = text[i]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        elif c == '"':
            i += 1
            while i < len(text) and text[i] != '"':
                i += 2 if text[i] == "\\" else 1
        i += 1
    return text[start : i - 1]


def strip_comments(s: str) -> str:
    s = re.sub(r"/\*.*?\*/", "", s, flags=re.S)
    return re.sub(r"//[^\n]*", "", s)


def scan(text: str, name: str) -> list[str]:
    out = []
    for m in HEAD.finditer(text):
        attrs = m.group(1)
        if not re.search(r"#\[(?:\w+::)?test\b", attrs) or not re.search(r"#\[ignore\b", attrs):
            continue
        body = strip_comments(body_at(text, m.end())).strip()
        if STUB.match(body):
            line = text.count("\n", 0, m.start(2)) + 1
            out.append(
                f"{name}:{line}: ignored test `{m.group(2)}` only panics: write it or delete it"
            )
    return out


def problems(root: Path) -> list[str]:
    out = []
    for d in DIRS:
        for f in sorted((root / d).rglob("*.rs")):
            if "/target/" in f.as_posix():
                continue
            out += scan(f.read_text(errors="ignore"), str(f.relative_to(root)))
    return out


def self_test() -> int:
    bad = [
        '#[test]\n#[ignore = "later"]\nfn a() {\n    panic!("not wired yet");\n}\n',
        "#[test]\n#[ignore]\nfn b() {\n    todo!()\n}\n",
        "#[ignore]\n#[test]\nfn c() { unimplemented!(); }\n",
        '/// doc\n#[tokio::test]\n#[ignore = "x"]\nasync fn d() {\n    // soon\n    panic!("{}", 1);\n}\n',
        '#[test]\n#[ignore]\nfn e() -> Result<(), ()> {\n    todo!("x (y)")\n}\n',
    ]
    good = [
        '#[test]\n#[ignore = "slow"]\nfn a() {\n    assert_eq!(1, 1);\n    panic!("x");\n}\n',
        '#[test]\nfn b() {\n    panic!("not ignored, so it runs and fails");\n}\n',
        '#[test]\n#[ignore]\nfn c() {\n    if std::env::var("X").is_err() { todo!() }\n}\n',
        '#[ignore = "manual"]\nfn helper() {\n    todo!()\n}\n',
    ]
    ok = all(scan(b, "bad") for b in bad) and not any(scan(g, "good") for g in good)
    print("self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    root = Path(args[0]).resolve() if args else ROOT
    bad = problems(root)
    print("\n".join(bad) or "stubs: ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
