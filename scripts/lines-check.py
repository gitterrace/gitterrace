#!/usr/bin/env python3
"""Source hygiene guard (stdlib only), run by `make lines`, part of `make check`.

1. No source file is longer than 600 lines.
2. No code or test comment points at a design doc section or a process label.

  python3 scripts/lines-check.py              check the tree (tracked and untracked, not ignored)
  python3 scripts/lines-check.py --self-test  prove both rules go red and green on fixtures
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MAX_LINES = 600
# rustfmt max_width. rustfmt never wraps a comment, so a wide one is caught here.
MAX_WIDTH = 100
LENGTH_GLOBS = ("*.rs", "*.py", "*.mjs", "*.js", "*.sh", "*.css", "*.html")
COMMENT_EXTS = (".rs", ".py", ".mjs", ".js", ".sh")
# Vendored third-party files and the synced theme copy are not ours to split or reword.
EXCLUDE = ("static/vendor/", "theme/schwarzwald/")

# Process labels in comments. Percent-encodings such as %C3 sit in code, not in comments, and
# the whole-word match keeps a label inside longer tokens out.
LABELS = re.compile(
    r"§"
    r"|\b[Ss]ection \d+\.\d+"
    r"|\bdesign (doc|section)"
    r"|\b[Ll]ane [A-Z]\b"
    r"|\b(C[1-3]|K[1-5]|N[1-4])\b"
)
# The local, gitignored denylist (CONV-PERSONAL-DATA reads it too): plain names here are refused
# as people in comments. Regex lines and six-hex ids are the other scripts' part.
LOCAL = ROOT / ".personal-denylist"


def local_names() -> list[str]:
    try:
        lines = LOCAL.read_text().splitlines()
    except OSError:
        return []
    return [
        re.escape(n.strip())
        for n in lines
        if re.fullmatch(r"[A-Za-z][\w-]*", n.strip())
        and not re.fullmatch(r"[0-9a-f]{6}", n.strip())
    ]


def person_pattern(names: list[str]) -> re.Pattern[str] | None:
    """A name counts when capitalised, or lowercase before a colon, after @, `by`, `ask`, `told`."""
    if not names:
        return None
    alt = "|".join(names)
    cap = "|".join(n[0].upper() + n[1:] for n in names)
    return re.compile(rf"\b({cap})\b|\b({alt}):|(?:@|\bby |\bask |\btold )({alt})\b")


# `//` and `#` start a comment after whitespace or at line start. `://` and `#[` are not.
COMMENT = re.compile(r"(?:^|\s)(?://+!?|#)(?=\s|$)(.*)$|^\s*\*(?!/)\s?(.*)$")


def files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "-co", "--exclude-standard", "--", *LENGTH_GLOBS],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return sorted(f for f in out.split("\0") if f and not any(x in f for x in EXCLUDE))


def too_long(root: Path, names: list[str]) -> list[str]:
    bad = []
    for n in names:
        try:
            k = len((root / n).read_text(errors="replace").splitlines())
        except OSError:
            continue
        if k > MAX_LINES:
            bad.append(f"{n}: {k} lines, limit {MAX_LINES}")
    return bad


# Wide lines that predate the width rule: strings and macro bodies rustfmt leaves alone. A
# count may only go down: a new wide line fails, and a fixed one must lower its count here.
WIDE_ALLOWED: dict[str, int] = {
    "crates/gitterrace-actions/src/config/tests.rs": 6,
    "crates/gitterrace-actions/src/payload/tests.rs": 1,
    "crates/gitterrace-actions/src/runner.rs": 1,
    "crates/gitterrace-actions/src/schema.rs": 10,
    "crates/gitterrace-api/src/endpoints.rs": 2,
    "crates/gitterrace-api/src/insights.rs": 4,
    "crates/gitterrace-api/src/mcp.rs": 2,
    "crates/gitterrace-cards/src/lib.rs": 1,
    "crates/gitterrace-cards/src/wrapped.rs": 8,
    "crates/gitterrace-config/src/lib.rs": 2,
    "crates/gitterrace-config/src/schema.rs": 7,
    "crates/gitterrace-index/src/insights/mod.rs": 1,
    "crates/gitterrace-index/src/insights/stats.rs": 1,
    "crates/gitterrace-index/src/lib.rs": 6,
    "crates/gitterrace-index/src/store.rs": 2,
    "crates/gitterrace-index/tests/it/count_parity_fuzz.rs": 9,
    "crates/gitterrace-testkit/src/lib.rs": 2,
    "crates/gitterrace-testkit/src/oracle/tests.rs": 1,
    "crates/gitterrace-types/src/codec.rs": 1,
    "crates/gitterrace-web/src/actions/tests.rs": 1,
    "crates/gitterrace-web/src/charts.rs": 12,
    "crates/gitterrace-web/src/commit_types.rs": 4,
    "crates/gitterrace-web/src/error.rs": 1,
    "crates/gitterrace-web/src/features_tests.rs": 1,
    "crates/gitterrace-web/src/insights.rs": 1,
    "crates/gitterrace-web/src/languages/render.rs": 12,
    "crates/gitterrace-web/src/lib.rs": 3,
    "crates/gitterrace-web/src/markdown.rs": 5,
    "crates/gitterrace-web/src/markdown/lines.rs": 1,
    "crates/gitterrace-web/src/markdown/outline.rs": 1,
    "crates/gitterrace-web/src/media.rs": 4,
    "crates/gitterrace-web/src/overview_side.rs": 1,
    "crates/gitterrace-web/src/routes/assets.rs": 2,
    "crates/gitterrace-web/src/routes/files.rs": 1,
    "crates/gitterrace-web/src/sitemap.rs": 4,
    "crates/gitterrace-web/src/state_tests.rs": 1,
    "crates/gitterrace-web/src/views/code.rs": 1,
    "crates/gitterrace-web/src/views/diff.rs": 3,
    "crates/gitterrace-web/src/views/diff/tests.rs": 1,
    "crates/gitterrace-web/src/when.rs": 1,
    "crates/gitterrace-web/src/wiring_live_tests.rs": 2,
    "crates/gitterrace-web/src/wiring_tests.rs": 1,
    "crates/gitterrace-web/tests/doc_cards_share_one_header.rs": 2,
    "crates/gitterrace-web/tests/memory_budget.rs": 2,
    "crates/gitterrace-web/tests/running_cache_rewritten.rs": 2,
    "crates/gitterrace-web/tests/sha_links.rs": 1,
    "crates/gitterrace-web/tests/theme.rs": 1,
    "crates/gitterrace/src/alloc.rs": 2,
    "crates/gitterrace/src/cli.rs": 1,
    "crates/gitterrace/src/export.rs": 2,
    "crates/gitterrace/src/export/credit.rs": 1,
    "crates/gitterrace/src/export/credit/tests.rs": 1,
    "crates/gitterrace/src/export/out.rs": 1,
    "crates/gitterrace/src/export/snapshot.rs": 1,
    "crates/gitterrace/src/handler.rs": 9,
    "crates/gitterrace/src/open.rs": 5,
    "crates/gitterrace/src/open/tests.rs": 1,
    "xtask/src/api_md.rs": 2,
    "xtask/src/config_md.rs": 2,
    "xtask/src/docs_md.rs": 2,
    "xtask/src/openapi_json.rs": 2,
    "xtask/src/readme_agents_md.rs": 1,
    "xtask/src/site.rs": 33,
    "xtask/src/site/shots.rs": 1,
    "xtask/src/site/tests.rs": 2,
}

RAW_OPEN = re.compile(r'r(#+)"')
URL = re.compile(r"\bhttps?://\S{40,}")


def wide_lines(text: str) -> list[int]:
    """Lines past MAX_WIDTH, except inside a raw string (fixture bytes such as SVG or TOML,
    where a wrap would change the content) and lines carrying one long URL."""
    out, close = [], None
    for i, line in enumerate(text.splitlines(), 1):
        inside = close is not None
        if close is not None and close in line:
            close = None
        else:
            for m in RAW_OPEN.finditer(line):
                end = '"' + m.group(1)
                if end not in line[m.end() :]:
                    close = end
                    break
        raw = inside or close is not None
        if len(line) > MAX_WIDTH and not raw and not URL.search(line):
            out.append(i)
    return out


def too_wide(
    root: Path, names: list[str], allowed: dict[str, int] | None = None
) -> list[str]:
    allowed = WIDE_ALLOWED if allowed is None else allowed
    bad = []
    for n in names:
        if not n.endswith(".rs"):
            continue
        try:
            text = (root / n).read_text(errors="replace")
        except OSError:
            continue
        rows = wide_lines(text)
        cap = allowed.get(n, 0)
        lines = text.splitlines()
        if len(rows) > cap:
            for i in rows:
                if cap == 0 or lines[i - 1].lstrip().startswith("//"):
                    bad.append(
                        f"{n}:{i}: {len(lines[i - 1])} columns, limit {MAX_WIDTH}"
                    )
            if cap:
                bad.append(
                    f"{n}: {len(rows)} wide lines, allowed {cap} (wrap the new one)"
                )
        elif len(rows) < cap:
            bad.append(
                f"{n}: {len(rows)} wide lines, allowed {cap}: lower WIDE_ALLOWED"
            )
    return bad


def label_hits(
    root: Path, names: list[str], extra: list[str] | None = None
) -> list[str]:
    person = person_pattern(local_names() if extra is None else extra)
    bad = []
    for n in names:
        if not n.endswith(COMMENT_EXTS) or n == "scripts/lines-check.py":
            continue
        try:
            lines = (root / n).read_text(errors="replace").splitlines()
        except OSError:
            continue
        for i, line in enumerate(lines, 1):
            m = COMMENT.search(line)
            if not m:
                continue
            text = m.group(1) if m.group(1) is not None else m.group(2) or ""
            if LABELS.search(text) or (person and person.search(text)):
                bad.append(f"{n}:{i}: process label in a comment: {text.strip()[:70]}")
    return bad


def run(root: Path, names: list[str]) -> list[str]:
    return too_long(root, names) + too_wide(root, names) + label_hits(root, names)


def self_test() -> None:
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "ok600.rs").write_text("fn a() {}\n" * 600)
        (root / "big601.rs").write_text("fn a() {}\n" * 601)
        assert too_long(root, ["ok600.rs"]) == [], "600 lines must pass"
        assert len(too_long(root, ["big601.rs"])) == 1, "601 lines must fail"
        ok = "    /// " + "x" * 92 + "\n//! " + "y" * 96 + "\n"
        (root / "w.rs").write_text(ok + 'let s = r#"\n' + "z" * 140 + '\n"#;\n')
        (root / "w2.rs").write_text("    // " + "x" * 94 + "\n")
        (root / "w3.rs").write_text('    let s = "' + "x" * 95 + '";\n')
        assert too_wide(root, ["w.rs"], {}) == [], "100 columns and raw fixtures pass"
        assert len(too_wide(root, ["w2.rs"], {})) == 1, "a 101 column comment fails"
        assert len(too_wide(root, ["w3.rs"], {})) == 1, "a 101 column string fails"
        assert too_wide(root, ["w3.rs"], {"w3.rs": 1}) == [], "an allowed count passes"
        assert too_wide(root, ["w.rs"], {"w.rs": 1}), (
            "an allowance above the count fails"
        )
        green = [
            'let s = "%C3%A9"; // encodes e acute',
            'let u = "http://x"; // plain',
            "# a plain word in prose",
            "// Lane is a word, lanes too",
            "#[derive(Debug)]",
            "// the case_name field",
        ]
        red = [
            "// see design doc 4",
            "// per section 3.2",
            "// signed off as K3",
            "// lane Q owns this",
            "// signed off as N2",
            "// \u00a7 4",
        ]
        (root / "g.rs").write_text("\n".join(green) + "\n")
        assert label_hits(root, ["g.rs"]) == [], label_hits(root, ["g.rs"])
        # The optional local name list is the only place a person name can come from.
        (root / "n.rs").write_text("// ask Orca\n// orca: later\n// the orca swims\n")
        assert label_hits(root, ["n.rs"], []) == [], "no list, no name rule"
        assert len(label_hits(root, ["n.rs"], ["orca"])) == 2, "listed name must flag"
        for i, r in enumerate(red):
            (root / f"r{i}.rs").write_text(r + "\n")
            assert label_hits(root, [f"r{i}.rs"]), f"should flag: {r}"
    print("lines-check self-test: ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    bad = run(ROOT, files())
    for b in bad:
        print(b)
    if bad:
        print(f"lines-check: {len(bad)} problem(s)")
        return 1
    print("lines-check: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
