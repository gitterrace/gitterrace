"""What every shared lint reads: the product root, its lint.toml, the deny patterns and the
local denylist. Data only: nothing here runs product code.

The product root is `--repo DIR` when given, else the git top level of the working directory.
The config is `<root>/config/schwarzwald/lint.toml`, absent keys take the defaults below (the
same values `lint/config.example.toml` states). The deny patterns are `rules/deny.toml` next to
this part, so the copy in a product reads the copy beside it.
"""

from __future__ import annotations

import fnmatch
import re
import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ImportError:  # pragma: no cover
    sys.exit(f"the shared lints need python 3.11 or newer for tomllib, this is {sys.version.split()[0]}")

PART = Path(__file__).resolve().parents[2]  # the folder holding lint/, rules/, hooks/
DENY_FILE = PART / "rules" / "deny.toml"
CONFIG = Path("config/schwarzwald/lint.toml")

DEFAULTS: dict[str, dict] = {
    "product": {"name": "product"},
    "scope": {
        "prose": ["README.md", "AGENTS.md", "docs/**.md", "bench/**.md"],
        "code": ["*.rs", "*.py", "*.mjs", "*.js", "*.sh", "*.yml", "*.yaml", "*.toml"],
        "make": ["Makefile"],
        "design": ["docs/design/**.md"],
        "golden": ["*/golden/*"],
        "exclude": ["shared/*", "target/*"],
        "min_files": 1,
    },
    "refs": {"patterns": [], "not_id": []},
    "personal": {"exempt": [], "placeholder_homes": []},
    "history": {"keep": [], "sha_fields": [], "bundles": "", "rewrites": [], "skip": []},
    "commit": {
        "types": ["feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert"],
        "allow_prefix": "SCHWARZWALD_",
    },
}


def take_repo(args: list[str]) -> tuple[Path, list[str]]:
    """Pull `--repo DIR` out of args; default is the git top level of the working directory."""
    if "--repo" in args:
        i = args.index("--repo")
        root, args = Path(args[i + 1]).resolve(), args[:i] + args[i + 2 :]
    else:
        out = subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True)
        root = Path(out.stdout.strip() or ".").resolve()
    return root, args


def load(root: Path) -> dict[str, dict]:
    path = root / CONFIG
    got = tomllib.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
    unknown = [f"[{t}] {k}" for t, v in got.items() for k in v if k not in DEFAULTS.get(t, {})]
    if unknown:
        sys.exit(f"{path}: keys no shared lint reads: {', '.join(unknown)}")
    return {t: {**d, **got.get(t, {})} for t, d in DEFAULTS.items()}


def deny(product: str) -> dict[str, list[re.Pattern[str]]]:
    """deny.toml compiled, `{product}` filled in. `not_id` is the English the id patterns skip."""
    raw = tomllib.loads(DENY_FILE.read_text(encoding="utf-8"))
    out = {t: [re.compile(p.replace("{product}", re.escape(product))) for p in v["patterns"]] for t, v in raw.items()}
    out["not_id"] = [re.compile(v["not_id"]) for v in raw.values() if "not_id" in v]
    return out


def denylist(root: Path) -> tuple[list[re.Pattern[str]], set[str]]:
    """The local `.personal-denylist`: regexes, and the bare six-hex lines as a set of ids."""
    path = root / ".personal-denylist"
    if not path.exists():
        print("note: .personal-denylist absent, generic patterns only", file=sys.stderr)
        return [], set()
    pats, ids = [], set()
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if re.fullmatch(r"[0-9a-f]{6}", line):
            ids.add(line)
        else:
            pats.append(re.compile(line, re.IGNORECASE))
    return pats, ids


def matches(path: str, globs: list[str]) -> bool:
    return any(fnmatch.fnmatch(path, g) or fnmatch.fnmatch(path, g.replace("**", "*")) for g in globs)


def tracked(root: Path, exclude: list[str]) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        capture_output=True, text=True, check=True,
    ).stdout
    return [f for f in out.split("\0") if f and not matches(f, exclude)]


def messages(root: Path, base: str) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(root), "log", "-z", "--format=%B", f"{base}..HEAD"],
        capture_output=True, text=True, check=True,
    ).stdout
    return [m for m in out.split("\0") if m.strip()]


def message_lines(msg: str) -> list[str]:
    """Subject and body; lines git strips (starting with `#`) are skipped."""
    return [x for x in msg.splitlines() if not x.startswith("#")]
