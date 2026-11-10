#!/usr/bin/env python3
"""Naming guard (stdlib only), run by `make names`.

1. No references to other projects' internals (OUTSIDE_HASHES), outside the vendored
   theme snapshot and Cargo.lock.
2. Prose describes gitterrace on its own terms: no hosting product names (PRODUCT_NAMES)
   in docs, comments or descriptions. Code may still name them as data, such as a bot
   address, and release config may name its target.
3. Make targets are `<area>[-<verb>]`, lowercase words joined by single hyphens.
4. File and module names (R1 to R6 below): no catch-all stems, page modules named for their
   route, template partials marked with `_`, one name per insight, scripts as
   `<area>-<verb>` with a closed verb list, a header on every script, and generators whose
   header names what they write.
"""

from __future__ import annotations

import hashlib
import re
import subprocess
import sys
from collections.abc import Callable
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# Policy list of outside project names: this repo stands on its own and must not lean on
# another repo's internals. Not a ban on any deleted name. Stored as SHA-256 prefixes, so
# the guard does not itself spell the names it keeps out.
OUTSIDE_HASHES = frozenset(
    {
        "b6598e838f350a97",
        "82ba7e1f6f4171d3",
        "913b41407d461651",
        "86d3da4285747570",
    }
)
# .mcp.json is local agent tooling config, not product content. The guard shims and guards.env
# bind the shared guards to the agent harness, so they name its adapters and scratch dir.
SKIP = (
    "schwarzwald/kit/",
    "Cargo.lock",
    ".mcp.json",
    "scripts/lint/names-check.py",
    "scripts/guards/adapters/",
    "schwarzwald/config/guards.env",
)
WORD = re.compile(r"[A-Za-z_]{4,}")


def outside(line: str) -> str | None:
    """The first word of the line whose lowercase SHA-256 prefix is an outside name."""
    for w in WORD.findall(line):
        if hashlib.sha256(w.lower().encode()).hexdigest()[:16] in OUTSIDE_HASHES:
            return w
    return None


# Hosting products gitterrace is not, and need not be compared to.
PRODUCT_NAMES = ("GitHub", "GitLab", "Gitea", "Forgejo", "Bitbucket")
PRODUCT = re.compile(r"\b(" + "|".join(PRODUCT_NAMES) + r")\b", re.I)
# Editors are named as products only when capitalised; the lowercase name is a binary on PATH.
EDITORS = re.compile(r"\b(Zed|VS Code|Visual Studio Code|IntelliJ)\b")
# Where prose lives: Markdown, comments, and description lines in manifests. A URL or
# an email address is data, not a comparison.
PROSE_FILES = (".md",)
COMMENT = re.compile(r"^\s*(//|#|/\*|\*)|\s//\s|\bdescription\s*=|^\s*keywords\s*=")
DATA = re.compile(
    r"[\w.-]*\.(com|io|test)\b|@[\w.-]+|\"[\w-]+-actions\"|\b(github|gitlab|forgejo)-[a-z-]+\b"
)
# Release tooling must name its CI host.
PRODUCT_SKIP = ("Cargo.toml", ".github/")
# Third-party code shipped as is.
VENDORED = "/vendor/"


# R1: stems that say nothing about what the module holds.
CATCH_ALL = {"misc", "util", "utils", "helpers", "common", "other", "pages", "stuff"}
SRC_DIRS = ("crates/*/src", "xtask/src")
TEMPLATES = "crates/gitterrace-web/templates"
STEM = re.compile(r"^_?[a-z][a-z0-9_]*$")
# R5: `-lib` marks a library imported by other scripts, not a command.
SCRIPT = re.compile(
    r"^[a-z]+(-[a-z]+)*-(check|gen|run|export|report|render|lib)\.(py|sh|mjs)$"
)
HEADER = ("//", "#", '"""')
MAKE_TARGET = re.compile(r"^[a-z]+(-[a-z]+)?$")


def make_targets() -> list[str]:
    """Names declared in .PHONY, the Makefile's list of real targets (one line per section,
    backslash continuations joined)."""
    text = (ROOT / "Makefile").read_text().replace("\\\n", " ")
    out: list[str] = []
    for m in re.finditer(r"^\.PHONY:(.*)$", text, re.M):
        out += m.group(1).split()
    return out


def first_line(f: Path) -> str:
    lines = [ln for ln in f.read_text().splitlines() if not ln.startswith("#!")]
    return lines[0].strip() if lines else ""


def rust_files() -> list[Path]:
    return sorted(f for d in SRC_DIRS for f in ROOT.glob(d + "/**/*.rs"))


def check_modules() -> list[str]:
    """R1 and R2: module names say what the module holds."""
    bad = []
    for f in rust_files():
        rel = f.relative_to(ROOT)
        if f.stem in CATCH_ALL or any(p in CATCH_ALL for p in rel.parts[:-1]):
            bad.append(f"{rel}: catch-all module name, name it for its subject (R1)")
        if f.name == "types.rs":
            text = f.read_text()
            if "#[template" in text or "fn page" in text:
                bad.append(
                    f"{rel}: types.rs holds a page, name the module for its route (R2)"
                )
    return bad


def check_templates() -> list[str]:
    """R3 and R4: partials start with `_`, pages do not, and a route has one name."""
    bad = []
    tdir = ROOT / TEMPLATES
    stems = {f.stem for f in tdir.glob("*.html")}
    for st in sorted(stems):
        if not STEM.match(st):
            bad.append(f"{TEMPLATES}/{st}.html: template name must be snake_case (R3)")
    src = ROOT / "crates/gitterrace-web"
    for f in sorted([*src.glob("src/**/*.rs"), *tdir.glob("*.html")]):
        for m in re.finditer(r'#\[template\(path = "([^"]+)"', f.read_text()):
            if m.group(1).startswith("_"):
                bad.append(
                    f"{f.relative_to(ROOT)}: page template {m.group(1)} has a partial's `_` (R3)"
                )
        if f.suffix == ".html":
            for m in re.finditer(
                r'\{%-?\s*(?:include|import|extends)\s+"([^"]+)"', f.read_text()
            ):
                if not m.group(1).startswith("_"):
                    bad.append(
                        f"{f.relative_to(ROOT)}: {m.group(1)} is used as a partial, name it _{m.group(1)} (R3)"
                    )
    routes = (src / "src/routes.rs").read_text()
    insights = (ROOT / "crates/gitterrace-api/src/insights.rs").read_text()
    ids = set(re.findall(r'name: "([a-z_]+)"', insights))
    # A route that only redirects (to a section of another page) renders no template of its own.
    for seg in re.findall(r'\.route\("/stats/([a-z-]+)",\s*get\((?!moved\))', routes):
        snake = seg.replace("-", "_")
        want = snake
        if want not in stems:
            bad.append(
                f"/stats/{seg}: no template {want}.html, route and template share one name (R4)"
            )
        for i in sorted(ids):
            if i != snake and (i.endswith("_" + snake) or i.startswith(snake + "_")):
                bad.append(
                    f"/stats/{seg}: insight `{i}` names the same thing differently (R4)"
                )
    return bad


def script_files() -> list[Path]:
    """Every script under scripts/, its group dirs too. Hooks, guard adapters, ast-grep rules
    and the screenshot config are data or entry points other tools name, not scripts."""
    data = ("guards", "hooks", "ast-grep", "site-shots")
    return sorted(
        f
        for f in (ROOT / "scripts").rglob("*")
        if f.is_file()
        and not set(f.relative_to(ROOT / "scripts").parts[:-1]) & set(data)
        and "__pycache__" not in f.parts
    )


def check_headers() -> list[str]:
    """R5: scripts are `<area>-<verb>` and every script and static file opens with a comment."""
    bad = []
    for f in script_files():
        if not SCRIPT.match(f.name):
            bad.append(
                f"{f.relative_to(ROOT)}: not <area>-<verb> with a verb of check|gen|run|export|report|render|lib (R5)"
            )
    files = script_files()
    files += sorted((ROOT / "crates/gitterrace-web/static").glob("*.js"))
    files += sorted((ROOT / "xtask/src").glob("*.rs"))
    for f in files:
        if not first_line(f).startswith(HEADER):
            bad.append(
                f"{f.relative_to(ROOT)}: first line is not a header comment (R5)"
            )
    return bad


def check_generators() -> list[str]:
    """R6: a generator's `//!` header names every file it writes."""
    bad = []
    for f in sorted((ROOT / "xtask/src").glob("*_md.rs")) + [
        ROOT / "xtask/src/readme_agents_md.rs"
    ]:
        text = f.read_text()
        head = "\n".join(ln for ln in text.splitlines() if ln.startswith("//!"))
        body = text.split("#[cfg(test)]")[0]
        for path in sorted(set(re.findall(r'"((?:[\w.-]+/)*[A-Z\w.-]+\.md)"', body))):
            if path not in head:
                bad.append(
                    f"xtask/src/{f.name}: header does not name {path}, which it writes (R6)"
                )
    return bad


# Paths a script, generator or the Makefile names under these roots must exist.
PATH_REF = re.compile(
    r"(?<![\w./-])((?:crates|scripts|docs|xtask|demo|theme|site|styles)/[\w./-]*\w\.\w+)"
)
# Paths a generator writes or a doc names as an example are not references.
PATH_REF_SKIP = ("site/", "target/", "docs/brand/")


def path_ref_sources() -> list[Path]:
    """The files whose paths must resolve: every script (nested dirs too), the generators, the
    Makefile, and the configs and workflows that call scripts by path."""
    tracked = subprocess.run(
        ["git", "ls-files", "-z", "scripts", "schwarzwald/config", ".github"],
        cwd=ROOT,
        capture_output=True,
        check=True,
    ).stdout.decode()
    keep = (".py", ".sh", ".mjs", ".toml", ".env", ".yml", ".yaml")
    out = [ROOT / r for r in tracked.split("\0") if r.endswith(keep)]
    # The generators at the top of xtask/src; its submodules hold test fixtures of made-up paths.
    out += [
        *sorted((ROOT / "xtask/src").glob("*.rs")),
        ROOT / "Makefile",
        ROOT / "package.json",
    ]
    return [f for f in out if f.is_file() and "/tests/" not in f.as_posix()]


def stale_paths(text: str, exists: Callable[[str], bool]) -> list[tuple[int, str]]:
    """(line, path) for each path under PATH_REF in `text` that `exists` says is gone. Test
    fixtures (after #[cfg(test)]) and a toml [retired] table name gone paths on purpose."""
    body = re.split(r"^\[retired\]$", text.split("#[cfg(test)]")[0], flags=re.M)[0]
    out = []
    for n, line in enumerate(body.splitlines(), 1):
        for path in PATH_REF.findall(line):
            if "*" in path or "{" in path or path.startswith(PATH_REF_SKIP):
                continue
            if not exists(path):
                out.append((n, path))
    return out


def check_path_refs() -> list[str]:
    """R7: a renamed or moved file leaves no stale path in the scripts, generators, Makefile,
    configs or workflows."""
    return [
        f"{f.relative_to(ROOT)}:{n}: names {path}, which does not exist (R7)"
        for f in path_ref_sources()
        for n, path in stale_paths(f.read_text(), lambda p: (ROOT / p).exists())
    ]


def self_test() -> int:
    """R7 red and green: a moved script leaves its old path in a gate config."""
    have = {"scripts/new/x-check.py"}
    cases = [
        ('gate = ["python3 scripts/x-check.py"]\n', True),
        ('gate = ["python3 scripts/new/x-check.py"]\n', False),
        ('ok = 1\n[retired]\nentries = ["scripts/x-check.py"]\n', False),
        ("run: bash scripts/gone-check.sh\n", True),
    ]
    for text, red in cases:
        if bool(stale_paths(text, have.__contains__)) != red:
            print(f"names self-test: {'missed' if red else 'flagged'} {text!r}")
            return 1
    print("names self-test: ok")
    return 0


# A Rust version written into a build file: `+1.98`, `toolchain: "1.98"`, `channel = "1.98.0"`.
TOOLCHAIN_LITERAL = re.compile(
    r"(?:\+|toolchain:\s*[\"']?|channel\s*=\s*\")(1\.\d+(?:\.\d+)?)"
)


def check_toolchain() -> list[str]:
    """One compiler pin: rust-toolchain.toml. The Makefile and workflows read it, never repeat
    it, so a bump cannot leave one of them on the old compiler. The only other version allowed
    is the MSRV job's, and it must be the workspace `rust-version`."""
    msrv = re.search(
        r'^rust-version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M
    )
    allowed = {msrv.group(1)} if msrv else set()
    files = [ROOT / "Makefile", *sorted((ROOT / ".github/workflows").glob("*.yml"))]
    bad = []
    for f in files:
        for n, line in enumerate(f.read_text().splitlines(), 1):
            if line.lstrip().startswith("#"):
                continue
            for v in TOOLCHAIN_LITERAL.findall(line):
                if v not in allowed:
                    bad.append(
                        f"{f.relative_to(ROOT)}:{n}: Rust {v} written in, read rust-toolchain.toml"
                    )
    return bad


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bad = [
        *check_modules(),
        *check_templates(),
        *check_headers(),
        *check_generators(),
        *check_path_refs(),
        *check_toolchain(),
    ]
    bad += [
        f"Makefile: target `{t}` is not <area>[-<verb>]"
        for t in make_targets()
        if not MAKE_TARGET.match(t)
    ]
    tracked = subprocess.run(
        ["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True
    ).stdout
    for rel in tracked.decode().split("\0"):
        f = ROOT / rel
        if (
            not rel
            or rel.startswith(SKIP)
            or not f.is_file()
            or VENDORED in rel
            or f.suffix in {".woff2", ".png"}
        ):
            continue
        try:
            text = f.read_text()
        except UnicodeDecodeError:
            continue
        prose_file = rel.endswith(PROSE_FILES)
        for n, line in enumerate(text.splitlines(), 1):
            m = outside(line)
            if m:
                bad.append(f"{rel}:{n}: outside project name: {m}")
            if (
                rel in PRODUCT_SKIP
                or rel.startswith(".github/")
                or not (prose_file or COMMENT.search(line))
            ):
                continue
            m = PRODUCT.search(DATA.sub("", line)) or EDITORS.search(line)
            if m:
                bad.append(
                    f"{rel}:{n}: product name in prose, describe gitterrace itself: {m.group(0)}"
                )
    for b in bad:
        print(b)
    if bad:
        return 1
    print("names: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
