"""What every shared lint reads: the product root, its lint.toml, the deny patterns and the
local denylist. Data only: nothing here runs product code.

The product root is `--repo DIR` when given, else the git top level of the working directory.
The config is `<root>/schwarzwald/config/lint.toml`, absent keys take the defaults below (the
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

    # execv keeps the pid: the flag stops a loop in this process, and a child (another pid) that
    # inherits it still re-execs.
    newer = None if os.environ.get(_REEXEC) == str(os.getpid()) else _newest_python311()
    if newer:
        os.environ[_REEXEC] = str(os.getpid())
        os.execv(newer, [newer, sys.argv[0], *sys.argv[1:]])
    sys.exit(f"the shared lints need python 3.11 or newer for tomllib, this is {sys.version.split()[0]}"
             " and no python3.11+ was found on PATH, in /opt/homebrew/bin or in /usr/local/bin")

PART = Path(__file__).resolve().parents[2]  # the folder holding lint/, rules/, hooks/
DENY_FILE = PART / "rules" / "deny.toml"
CONFIG = Path("schwarzwald/config/lint.toml")

DEFAULTS: dict[str, dict] = {
    "product": {"name": "product"},
    "scope": {
        "prose": ["README.md", "AGENTS.md", "docs/**.md", "bench/**.md"],
        "code": ["*.rs", "*.py", "*.mjs", "*.js", "*.sh", "*.yml", "*.yaml", "*.toml"],
        "make": ["Makefile"],
        "design": ["docs/design/**.md"],
        "golden": ["*/golden/*"],
        "exclude": ["schwarzwald/kit/*", "target/*"],
        "min_files": 1,
    },
    "a11y": {"docs": ["README.md", "docs/**.md"], "h1_required": True, "known": []},
    "refs": {"patterns": [], "not_id": []},
    # own-shas: a file of `allow` may name a build commit that a `record` file stores under `key`
    # (a path ending in `#NAME` reads only the generated block NAME). Nothing else is allowed.
    "own_shas": {"allow": [], "record": "", "key": "commit"},
    "personal": {"exempt": [], "placeholder_homes": []},
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
        "markdownlint": "",
    },
    "disk": {"floor_gib": 50},
    "root": {"allow": [], "scratch_free": [".agents"]},
    "denylist": {"max_age_hours": 168, "mode": "warn", "history_ids": "",
                 "path": "~/.config/schwarzwald/personal-denylist"},
    "commit": {
        "types": ["feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert"],
        "allow_prefix": "SCHWARZWALD_",
        "main": "main",
    },
}


def git(root: Path, *args: str, env_extra: dict[str, str] | None = None) -> str:
    """`git -C root args` with every inherited GIT_* removed: a hook runs the lints with GIT_DIR
    and GIT_INDEX_FILE set, which would point `git init` in a scratch dir at the real repository."""
    import os

    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")} | (env_extra or {})
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, check=True,
                          env=env).stdout


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
    cfg = merged(got)
    pin = cfg["run"]["markdownlint"]
    if pin and not PIN.fullmatch(str(pin)):
        # npx --yes installs any npm spec (a URL, npm:other) with no prompt: only a version.
        sys.exit(f"{path}: [run] markdownlint must be a version like 0.18.1, not {pin!r}")
    return cfg


PIN = re.compile(r"\d+\.\d+\.\d+")
# The lists that switch a lint off for a path. A branch that adds to one judges itself by rules
# it wrote, so the message gate compares them with the base.
WIDENING = [("scope", "exclude"), ("lines", "exclude"), ("personal", "exempt"), ("scripts", "exempt"),
            ("a11y", "known"), ("run", "shell_exclude"), ("run", "shellcheck_exclude")]


def merged(got: dict) -> dict[str, dict]:
    cfg = {t: {**d, **got.get(t, {})} for t, d in DEFAULTS.items()}
    # Zero files scanned is a broken census, never a pass.
    cfg["scope"]["min_files"] = max(1, int(cfg["scope"]["min_files"]))
    return cfg


def widened(root: Path, base: str) -> list[str]:
    """What lint.toml at HEAD switches off that it did not at BASE: a path added to an exclude
    or exempt list, a lower min_files. `<allow_prefix>ALLOW_WIDER_LINT=1` lets a reviewed change
    through."""
    def at(rev: str) -> dict[str, dict]:
        r = subprocess.run(["git", "-C", str(root), "show", f"{rev}:{CONFIG}"], capture_output=True, text=True)
        return merged(tomllib.loads(r.stdout) if r.returncode == 0 else {})
    old, new = at(base), at("HEAD")
    import os
    if os.environ.get(new["commit"]["allow_prefix"] + "ALLOW_WIDER_LINT") == "1":
        return []
    out = [f"lint.toml [{t}] {k} adds {x!r}" for t, k in WIDENING for x in new[t][k] if x not in old[t][k]]
    if new["scope"]["min_files"] < old["scope"]["min_files"]:
        out.append(f"lint.toml [scope] min_files drops from {old['scope']['min_files']} to {new['scope']['min_files']}")
    return out


# A group holding a quantifier, itself quantified: `(a+)+`. Backtracking on such a pattern
# doubles per character, so one line hangs the hook. Patterns from lint.toml and the private
# list are user text: refused at load. Lines are cut to MAX_LINE before a search.
NESTED = re.compile(r"\((?:[^()\\]|\\.)*[+*}](?:[^()\\]|\\.)*\)[+*{]")
MAX_LINE = 500


def user_pattern(p: str, where: str, flags: int = 0) -> re.Pattern[str]:
    if NESTED.search(p):
        sys.exit(f"{where}: {p!r} nests a quantifier in a quantified group, which can hang the lint")
    return re.compile(p, flags)


def deny(product: str) -> dict[str, list[re.Pattern[str]]]:
    """deny.toml compiled, `{product}` filled in. `not_id` is the English the id patterns skip."""
    raw = tomllib.loads(DENY_FILE.read_text(encoding="utf-8"))
    out = {t: [re.compile(p.replace("{product}", re.escape(product))) for p in v["patterns"]] for t, v in raw.items()}
    out["not_id"] = [re.compile(v["not_id"]) for v in raw.values() if "not_id" in v]
    return out


def refresh_file(path: Path) -> Path:
    """The private refresh command: `denylist-refresh.toml` beside the private denylist."""
    return path.parent / "denylist-refresh.toml"


def refresh_argv(path: Path) -> list[str] | None:
    """`argv = [...]` from the private refresh file, or None. Never from a tracked file: a branch
    controls its lint.toml, so a command there would run as whoever commits on that branch."""
    f = refresh_file(path)
    if not f.is_file() or f.is_symlink():
        return None
    argv = tomllib.loads(f.read_text(encoding="utf-8")).get("argv")
    if not (isinstance(argv, list) and argv and all(isinstance(a, str) and a for a in argv)):
        sys.exit(f"{f}: argv must be a non-empty list of strings")
    return argv


def denylist_problems(root: Path, cfg: dict[str, dict], now: float | None = None) -> tuple[list[str], bool]:
    """A missing or stale personal denylist as lines to print, and whether the mode refuses.

    The list is local and seeded once; the board grows and the guard goes blind without a sound.
    Settings: lint.toml [denylist] max_age_hours (0 = never stale), mode (warn|refuse|refresh),
    history_ids (a file the refresh merges in), path (the list itself, see denylist_path). The
    refresh command is private: `argv = [...]` in denylist-refresh.toml beside the list, run
    without a shell from the repository root. In mode "refresh" a missing or stale list is
    refreshed that way, and the check refuses only when the command fails or leaves the list
    missing or stale. The command's output is never printed: it may carry secrets."""
    import time

    s = cfg["denylist"]
    if s["mode"] not in ("warn", "refuse", "refresh"):
        sys.exit(f"lint.toml [denylist] mode must be warn, refuse or refresh, not {s['mode']!r}")
    path = denylist_path(root, cfg)
    argv = refresh_argv(path)
    how = (f" Refresh it: {' '.join(argv)}" if argv
           else f" Put the refresh command in {refresh_file(path)} as argv = [...].")

    def problem(at: float | None) -> str | None:
        if not path.exists():
            return f"the personal denylist {path} is missing: bare ids are not checked.{how}"
        age = ((at if at is not None else time.time()) - path.stat().st_mtime) / 3600
        if s["max_age_hours"] and age > s["max_age_hours"]:
            return f"the personal denylist {path} is {age:.0f}h old (max {s['max_age_hours']}h): new ids are not checked.{how}"
        return None

    found = problem(now)
    if found is None:
        return [], False
    if s["mode"] != "refresh":
        return [found], s["mode"] == "refuse"
    if not argv:
        return [found, f"[denylist] mode is refresh but {refresh_file(path)} names no command."], True
    try:
        r = subprocess.run(argv, cwd=root, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL)
        code = r.returncode
    except OSError as e:
        code = f"not runnable: {e.strerror}"
    if code != 0:
        return [found, f"The refresh failed (exit {code}); run {refresh_file(path)} by hand to see why."], True
    after = problem(None)
    if after is not None:
        return [after, "The refresh ran but did not renew the list."], True
    return ["The personal denylist was stale or missing and has been refreshed."], False


def denylist_path(root: Path, cfg: dict[str, dict] | None = None) -> Path:
    """The private denylist: lint.toml [denylist] path, `~` expanded, outside every repository.

    A path inside the product root is refused (exit): private data parked in a work tree is
    one `git add -f` or one sync away from a commit."""
    raw = (cfg or load(root))["denylist"]["path"]
    path = Path(raw).expanduser()
    if not path.is_absolute():
        sys.exit(f"lint.toml [denylist] path must be absolute or start with ~, not {raw!r}")
    here = root.resolve()
    if path.resolve() == here or here in path.resolve().parents:
        sys.exit(f"refused: the denylist path {path} lies inside the repository {here}. "
                 "Set [denylist] path to a location outside every repository.")
    return path


def denylist(root: Path) -> tuple[list[re.Pattern[str]], set[str]]:
    """The private denylist: regexes, and the bare six-hex lines as a set of ids."""
    path = denylist_path(root)
    if not path.exists():
        print("note: the personal denylist is absent, generic patterns only", file=sys.stderr)
        return [], set()
    pats, ids = [], set()
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if re.fullmatch(r"[0-9a-f]{6}", line):
            ids.add(line)
        else:
            pats.append(user_pattern(line, str(path), re.IGNORECASE))
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


def shallow(root: Path) -> str | None:
    """A refusal when the clone is shallow: a history lint there inspects one commit and passes.
    CI checks out with fetch-depth 0 and fetch-tags (ci/lint-and-test.yml)."""
    r = subprocess.run(["git", "-C", str(root), "rev-parse", "--is-shallow-repository"],
                       capture_output=True, text=True)
    if r.returncode != 0 or r.stdout.strip() != "false":
        return "the clone is shallow (or not a repository): fetch the full history and tags first"
    return None


def message_lines(msg: str) -> list[str]:
    """Subject and body, every line: `git commit -m` and `-F` keep `#` lines, so a committed
    message never skips them. The commit-msg hook strips comments itself when git will."""
    return msg.splitlines()
