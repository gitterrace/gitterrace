#!/usr/bin/env python3
"""Naming guard (stdlib only), run by `make names`.

1. No references to other projects' internals (OUTSIDE_NAMES), outside the vendored
   theme snapshot and Cargo.lock.
2. Prose describes gitterrace on its own terms: no hosting product names (PRODUCT_NAMES)
   in docs, comments or descriptions. Code may still name them as data, such as a bot
   address, and release config may name its target.
"""
import re, subprocess, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# Policy list of outside project names: this repo stands on its own and must not lean on
# another repo's internals. Not a ban on any deleted name.
OUTSIDE_NAMES = ("(?!)",)
# .mcp.json is local agent tooling config, not product content.
SKIP = ("theme/schwarzwald/", "Cargo.lock", ".mcp.json", "scripts/names-check.py")
OUTSIDE = re.compile(r"\b(" + "|".join(OUTSIDE_NAMES) + r")\b", re.I)
# Hosting products gitterrace is not, and need not be compared to.
PRODUCT_NAMES = ("GitHub", "GitLab", "Gitea", "Forgejo", "Bitbucket")
PRODUCT = re.compile(r"\b(" + "|".join(PRODUCT_NAMES) + r")\b", re.I)
# Editors are named as products only when capitalised; the lowercase name is a binary on PATH.
EDITORS = re.compile(r"\b(Zed|VS Code|Visual Studio Code|IntelliJ)\b")
# Where prose lives: Markdown, comments, and description lines in manifests. A URL or
# an email address is data, not a comparison.
PROSE_FILES = (".md",)
COMMENT = re.compile(r"^\s*(//|#|/\*|\*)|\s//\s|\bdescription\s*=|^\s*keywords\s*=")
DATA = re.compile(r"[\w.-]*\.(com|io|test)\b|@[\w.-]+|\"[\w-]+-actions\"|\b(github|gitlab|forgejo)-[a-z-]+\b")
# Release tooling must name its CI host.
PRODUCT_SKIP = ("Cargo.toml", ".github/")
# Third-party code shipped as is.
VENDORED = "/vendor/"


def main() -> int:
    bad = []
    tracked = subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True).stdout
    for rel in tracked.decode().split("\0"):
        f = ROOT / rel
        if not rel or rel.startswith(SKIP) or not f.is_file() or VENDORED in rel or f.suffix in {".woff2", ".png"}:
            continue
        try:
            text = f.read_text()
        except UnicodeDecodeError:
            continue
        prose_file = rel.endswith(PROSE_FILES)
        for n, line in enumerate(text.splitlines(), 1):
            m = OUTSIDE.search(line)
            if m:
                bad.append(f"{rel}:{n}: outside project name: {m.group(0)}")
            if rel in PRODUCT_SKIP or rel.startswith(".github/") or not (prose_file or COMMENT.search(line)):
                continue
            m = PRODUCT.search(DATA.sub("", line)) or EDITORS.search(line)
            if m:
                bad.append(f"{rel}:{n}: product name in prose, describe gitterrace itself: {m.group(0)}")
    for b in bad:
        print(b)
    if bad:
        return 1
    print("names: ok")
    return 0


sys.exit(main())
