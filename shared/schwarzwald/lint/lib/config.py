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

_REEXEC = "SCHWARZWALD_PY311_REEXEC"


def _newest_python311() -> str | None:
    """The newest `python3.N` with N >= 11 on PATH or in the Homebrew and /usr/local bins."""
    import os

    best: tuple[int, str] | None = None
    for d in [*os.environ.get("PATH", "").split(os.pathsep), "/opt/homebrew/bin", "/usr/local/bin"]:
        try:
            names = os.listdir(d)
        except OSError:
            continue
        for name in names:
            m = re.fullmatch(r"python3\.(\d+)", name)
            path = os.path.join(d, name)
            if m and int(m.group(1)) >= 11 and os.access(path, os.X_OK):
                if best is None or int(m.group(1)) > best[0]:
                    best = (int(m.group(1)), path)
    return best[1] if best else None


try:
    import tomllib
except ImportError:  # pragma: no cover - only on a python older than 3.11
    import os

    newer = None if os.environ.get(_REEXEC) == "1" else _newest_python311()
    if newer:
        os.environ[_REEXEC] = "1"
        os.execv(newer, [newer, sys.argv[0], *sys.argv[1:]])
    sys.exit(f"the shared lints need python 3.11 or newer for tomllib, this is {sys.version.split()[0]}"
             " and no python3.11+ was found on PATH, in /opt/homebrew/bin or in /usr/local/bin")

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
    "a11y": {"docs": ["README.md", "docs/**.md"], "h1_required": True, "known": []},
    "refs": {"patterns": [], "not_id": []},
    "personal": {"exempt": [], "placeholder_homes": []},
    "history": {"keep": [], "sha_fields": [], "bundles": "", "replace": [], "skip": []},
    "lines": {
        "max_lines": 600,
        "max_width": 0,
        "max_code_width": 0,
        "globs": ["*.rs", "*.py", "*.mjs", "*.js", "*.sh", "*.css", "*.html"],
        "exclude": [],
    },
    "scripts": {
        "dirs": ["scripts"],
        "readers": ["Makefile", "*.md", "*.yml", "*.yaml", "*.toml", "scripts/*"],
        "exempt": [],
    },
    "changelog": {"git_cliff": "", "file": "CHANGELOG.md", "tag_prefix": "v"},
    "run": {
        "steps": [],
        "shell_exclude": [],
        "shellcheck_exclude": ["SC2250", "SC2292", "SC2310", "SC2312"],
        "shfmt_indent": 2,
        "markdownlint": "",
        "markdown_exclude": [],
    },
    "disk": {"floor_gib": 50},
    "denylist": {"max_age_hours": 168, "mode": "warn", "refresh": "", "history_ids": ""},
    "commit": {
        "types": ["feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert"],
        "allow_prefix": "SCHWARZWALD_",
        "main": "main",
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


def denylist_problems(root: Path, cfg: dict[str, dict], now: float | None = None) -> tuple[list[str], bool]:
    """A missing or stale `.personal-denylist` as lines to print, and whether the mode refuses.

    The list is local and seeded once; the board grows and the guard goes blind without a sound.
    Settings: lint.toml [denylist] max_age_hours (0 = never stale), mode (warn|refuse|refresh),
    refresh (the product's refresh command), history_ids (a file the refresh merges in). In mode
    "refresh" a missing or stale list is refreshed by running that command from the repository
    root, and the check refuses only when the command fails or leaves the list missing or stale."""
    import subprocess
    import time

    s = cfg["denylist"]
    if s["mode"] not in ("warn", "refuse", "refresh"):
        sys.exit(f"lint.toml [denylist] mode must be warn, refuse or refresh, not {s['mode']!r}")
    path = root / ".personal-denylist"
    how = f" Refresh it: {s['refresh']}" if s["refresh"] else " Set [denylist] refresh in lint.toml to name the refresh command."

    def problem(at: float | None) -> str | None:
        if not path.exists():
            return f".personal-denylist is missing: bare ids are not checked.{how}"
        age = ((at if at is not None else time.time()) - path.stat().st_mtime) / 3600
        if s["max_age_hours"] and age > s["max_age_hours"]:
            return f".personal-denylist is {age:.0f}h old (max {s['max_age_hours']}h): new ids are not checked.{how}"
        return None

    found = problem(now)
    if found is None:
        return [], False
    if s["mode"] != "refresh":
        return [found], s["mode"] == "refuse"
    if not s["refresh"]:
        return [found, "[denylist] mode is refresh but refresh names no command."], True
    r = subprocess.run(s["refresh"], shell=True, cwd=root, capture_output=True, text=True)
    if r.returncode != 0:
        tail = (r.stderr or r.stdout).strip().splitlines()[-3:]
        return [found, f"The refresh failed (exit {r.returncode}): " + " | ".join(tail)], True
    after = problem(None)
    if after is not None:
        return [after, "The refresh ran but did not renew the list."], True
    return [".personal-denylist was stale or missing and has been refreshed."], False


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
