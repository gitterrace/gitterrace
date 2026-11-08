#!/usr/bin/env python3
"""Root guard (stdlib only), run by `make lint` and land-check.

Every tracked top-level entry must be on ALLOWED. A new file or folder at the root fails
here: put it under scripts/, tests/, docs/ or config/, or add it below with the tool that
needs it there. Entries are listed once, grouped by why they stay at the root.
"""

import subprocess
import sys

ALLOWED = {
    # Project documents
    "AGENTS.md",
    "CHANGELOG.md",
    "CONTRIBUTING.md",
    "LICENSE",
    "README.md",
    "SECURITY.md",
    # Cargo and toolchain, found by walking up from the workspace root
    ".cargo",
    "Cargo.lock",
    "Cargo.toml",
    "clippy.toml",
    "deny.toml",
    "rust-toolchain.toml",
    # Linter and formatter configs their tools read from the root
    ".gitleaks.toml",
    ".markdownlint-cli2.jsonc",
    ".prettierignore",
    ".prettierrc.json",
    ".vale.ini",
    "_typos.toml",
    "cliff.toml",
    "ruff.toml",
    "sgconfig.yml",
    "styles",
    # Editor and type checker configs, plus the web toolchain manifests
    "jsconfig.json",
    "pyrightconfig.json",
    "package.json",
    "package-lock.json",
    "wrangler.toml",
    # Git and CI
    ".github",
    ".gitignore",
    # Make, and the code and content folders
    "Makefile",
    "config",
    "crates",
    "docs",
    "scripts",
    "shared",
    "site",
    "tests",
    "xtask",
}


def main() -> int:
    out = subprocess.run(
        ["git", "ls-files", "-z"], check=True, capture_output=True, text=True
    ).stdout
    tops = {p.split("/", 1)[0] for p in out.split("\0") if p}
    stray = sorted(tops - ALLOWED)
    for t in stray:
        print(
            f"root-check: `{t}` is not an allowed top-level entry (see scripts/root-check.py)"
        )
    return 1 if stray else 0


if __name__ == "__main__":
    sys.exit(main())
