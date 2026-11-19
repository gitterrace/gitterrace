#!/usr/bin/env python3
"""Regex text lints, one table: each rule names the files it reads and the check it runs.

- status-words: two status facts run together without punctuation in user-visible strings
  ("on main clean" should read "on main, clean"). Rust string literals, templates and JS UI
  text. Not a full parser, it looks for a status word right after a name or a placeholder.
- history-comments: comments state what is true now, not what the code used to do.
- css-scope: every markdown.css selector is scoped to .markdown-body, .code or .diff. Rendered
  documents share the page with the app, so an unscoped class there (.seg once) restyles app
  widgets.

  python3 scripts/lint/text-check.py              check the tree
  python3 scripts/lint/text-check.py --self-test  every rule red and green on planted text
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections.abc import Callable, Iterator
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SELF = "scripts/lint/text-check.py"
COMMENT_EXTS = {".rs": "//", ".mjs": "//", ".py": "#", ".sh": "#"}
HISTORY = re.compile(r"\b(?:no longer|used to|previously|was changed)\b", re.I)
# Existing comments with history wording, `path:line` entries. Only shrinks.
ALLOW: set[str] = set()
EXTS = (".rs", ".html", ".js", ".mjs", ".md")
EXCLUDE = ("static/vendor/", "vendor/schwarzwald/kit/", "target/", "CHANGELOG.md")
MARKDOWN_CSS = "crates/gitterrace-web/static/markdown.css"
W = r"(?:clean|dirty|ahead|behind|detached|untracked|modified)"
PATTERNS = [
    # "on main clean", "branch {name} ahead 2"
    re.compile(
        rf"\b(?:on|branch) (?:\{{\{{?[^}}]*\}}\}}?|(?!(?:a|an|the|any|this|that|no)\b)[\w./-]+) {W}\b(?! (?:tree|checkout|state))"
    ),
    # "{branch} clean", "{{ name }} dirty"
    re.compile(rf"\{{[a-z_]+\}}\s+{W}(?=\s*(?:[\d\"'`<]|$))"),
    # template: a name, endif, then a bare status word span with no separator
    re.compile(rf"</(?:a|b)>(?:\{{% endif %\}}.*?)?\{{% else %\}} <span[^>]*>{W}<"),
]
SKIP = re.compile(r"^\s*(//|#|\*)")
SCOPED = re.compile(
    r"^(\.markdown-body|\.code|\.diff|:is\(\.markdown-body,\.code,\.diff\))(?![\w-])"
)


def status_words(text: str, name: str) -> list[str]:
    out = []
    for i, line in enumerate(text.splitlines(), 1):
        if SKIP.match(line):
            continue
        if any(p.search(line) for p in PATTERNS):
            out.append(
                f"{name}:{i}: status facts run together, add a comma: {line.strip()[:90]}"
            )
    return out


def history_comments(text: str, name: str) -> list[str]:
    marker = COMMENT_EXTS[Path(name).suffix]
    out = []
    for i, line in enumerate(text.splitlines(), 1):
        at = line.find(marker)
        if at < 0 or (marker == "#" and line.startswith("#!")):
            continue
        if at > 0 and line[:at].strip() != "" and f" {marker}" not in line[at - 1 :]:
            continue
        m = HISTORY.search(line[at:])
        if m and f"{name}:{i}" not in ALLOW:
            out.append(
                f"{name}:{i}: comment looks back ('{m.group(0)}'), say what holds now"
            )
    return out


def selectors(text: str) -> Iterator[str]:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    text = re.sub(r"url\([^)]*\)", "url()", text)
    for prelude in re.findall(r"(?:^|[};])\s*([^{};]+)\{", text):
        prelude = prelude.strip()
        if prelude.startswith("@"):
            continue
        depth, cur = 0, ""
        for ch in prelude:
            depth += ch in "(["
            depth -= ch in ")]"
            if ch == "," and depth == 0:
                yield cur.strip()
                cur = ""
            else:
                cur += ch
        yield cur.strip()


def css_scope(text: str, name: str) -> list[str]:
    return [
        f"{name}: selector not scoped to .markdown-body, .code or .diff: {s}"
        for s in selectors(text)
        if not SCOPED.match(s)
    ]


# rule -> (selects a tracked path, check)
RULES: dict[str, tuple[Callable[[str], bool], Callable[[str, str], list[str]]]] = {
    "status-words": (lambda f: f.endswith(EXTS + tuple(COMMENT_EXTS)), status_words),
    "history-comments": (lambda f: Path(f).suffix in COMMENT_EXTS, history_comments),
    "css-scope": (lambda f: f == MARKDOWN_CSS, css_scope),
}

# (rule, file name, text, red?)
CASES = [
    ("status-words", "x.rs", 'println!("on main clean");', True),
    ("status-words", "x.rs", 'format!("{branch} dirty")', True),
    ("status-words", "x.html", '</a>{% else %} <span class="muted">clean</span>', True),
    ("status-words", "x.rs", 'println!("on main, clean");', False),
    ("status-words", "x.rs", 'format!("{branch}, dirty")', False),
    ("status-words", "x.rs", "a clean checkout", False),
    ("history-comments", "x.rs", "// no longer needed", True),
    ("history-comments", "x.rs", "    /// used to be slow", True),
    ("history-comments", "x.rs", "let a = 1; // previously 2", True),
    ("history-comments", "x.py", "# was changed", True),
    ("history-comments", "x.rs", '// the cache holds now\nlet s = "no longer";', False),
    ("history-comments", "x.sh", "#!/bin/sh -e", False),
    ("css-scope", MARKDOWN_CSS, ".seg { color: x }", True),
    ("css-scope", MARKDOWN_CSS, ".markdown-body .seg, .code a { color: x }", False),
]


def files() -> list[str]:
    r = subprocess.run(
        ["git", "ls-files", "-c"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return [
        f
        for f in r.stdout.split("\n")
        if f and not any(x in f for x in EXCLUDE) and f != SELF
    ]


def self_test() -> int:
    for rule, name, text, red in CASES:
        if bool(RULES[rule][1](text, name)) != red:
            print(
                f"text-check self-test: {rule} {'passed' if red else 'failed'}: {text}"
            )
            return 1
    print("text-check self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    hits = []
    for f in files():
        checks = [check for picks, check in RULES.values() if picks(f)]
        if checks:
            text = (ROOT / f).read_text(errors="ignore")
            hits += [h for check in checks for h in check(text, f)]
    print("\n".join(hits) or "text-check: ok")
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
