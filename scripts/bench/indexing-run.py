#!/usr/bin/env python3
"""Keep desktop search indexers out of build, bench and agent directories (stdlib only).

Indexers rescanning cargo target dirs and bench corpora cost CPU and distort benches. This is the
ONE place that knows the directory list and how each OS excludes one. Other scripts call it.

  python3 scripts/bench/indexing-run.py --check            exit 1 and list dirs lacking the
                                                           exclusion
  python3 scripts/bench/indexing-run.py --apply            exclude every expected dir (idempotent)
  python3 scripts/bench/indexing-run.py --apply-dir PATH   create PATH if needed and exclude it
  python3 scripts/bench/indexing-run.py --list             print the expected dirs
  python3 scripts/bench/indexing-run.py --self-test        guard on a temp tree

Expected dirs: the repo's parent (the org root), target/, the bench cache (GITTERRACE_BENCH_DIR,
default ~/.cache/gitterrace-bench), ~/.cargo, ~/.rustup, and the
harness dirs that exist (~/.claude, ~/.codex, ~/.cursor, ~/.config/opencode, ~/.pi). Extras go in
GITTERRACE_INDEX_EXTRA, separated like PATH.

  macOS    a `.metadata_never_index` file in the dir or any ancestor. Spotlight reads it when it indexes the dir,
           so an already indexed tree may need `sudo mdutil -E <volume>` once, or the System
           Settings > Spotlight > Search Privacy list. The script never runs sudo.
  Windows  the not-content-indexed attribute (`attrib +I`, recursive on apply).
  Linux    a `.trackerignore` file (tracker3) and, when ~/.config/baloofilerc exists, an entry in
           its `exclude folders` (baloo). Other indexers get a printed instruction.
"""

from __future__ import annotations

import argparse
import os
import platform
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SYSTEM = platform.system()
MARKER = {"Darwin": ".metadata_never_index", "Linux": ".trackerignore"}.get(SYSTEM)
NOT_CONTENT_INDEXED = 0x2000
HARNESS = ["~/.claude", "~/.codex", "~/.cursor", "~/.config/opencode", "~/.pi"]


def bench_dir() -> Path:
    return Path(os.environ.get("GITTERRACE_BENCH_DIR", Path.home() / ".cache/gitterrace-bench"))


def bench_dirs() -> list[Path]:
    """What a timed bench touches. bench-run.py refuses to start unless these are excluded."""
    target = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    return [target, bench_dir()]


def expected() -> list[Path]:
    dirs = [ROOT.parent, *bench_dirs()]
    dirs += [Path(p).expanduser() for p in ("~/.cargo", "~/.rustup")]
    dirs += [p for p in map(Path, map(os.path.expanduser, HARNESS)) if p.is_dir()]
    dirs += [
        Path(p).expanduser()
        for p in os.environ.get("GITTERRACE_INDEX_EXTRA", "").split(os.pathsep)
        if p
    ]
    seen: dict[Path, None] = {}
    for d in dirs:
        seen.setdefault(d.absolute(), None)
    return list(seen)


def baloo_conf() -> Path:
    return Path.home() / ".config/baloofilerc"


def baloo_excludes() -> list[str]:
    for line in baloo_conf().read_text().splitlines() if baloo_conf().exists() else []:
        if line.startswith("exclude folders"):
            return [p.strip() for p in line.split("=", 1)[1].split(",") if p.strip()]
    return []


def excluded(d: Path, stop: Path | None = None) -> bool:
    """On macOS, tracker and baloo a dir counts when it or any ancestor is excluded, because the
    indexer skips the whole subtree. `stop` ends the walk (tests). Windows attributes are not inherited, so check the dir."""
    if SYSTEM == "Windows":
        if not d.is_dir():
            return False
        import ctypes

        attrs = ctypes.windll.kernel32.GetFileAttributesW(str(d))  # type: ignore[attr-defined]
        return attrs != -1 and bool(attrs & NOT_CONTENT_INDEXED)
    stop = stop or (
        Path(os.environ["GITTERRACE_INDEX_STOP"])
        if os.environ.get("GITTERRACE_INDEX_STOP")
        else None
    )
    baloo = baloo_excludes() if SYSTEM == "Linux" else []
    for p in (d, *d.parents):
        if stop and not p.is_relative_to(stop):
            break
        if MARKER and (p / MARKER).exists():
            return True
        if str(p) in baloo:
            return True
    return False


def exclude(d: Path) -> None:
    d.mkdir(parents=True, exist_ok=True)
    if SYSTEM == "Windows":
        subprocess.run(["attrib", "+I", str(d), "/S", "/D"], check=False, capture_output=True)
        subprocess.run(["attrib", "+I", str(d)], check=False, capture_output=True)
        return
    if MARKER:
        (d / MARKER).touch()
    if SYSTEM == "Linux" and baloo_conf().exists() and str(d) not in baloo_excludes():
        lines = baloo_conf().read_text().splitlines()
        new = str(d)
        for i, line in enumerate(lines):
            if line.startswith("exclude folders"):
                lines[i] = (
                    line.rstrip().rstrip(",") + "," + new
                    if "=" in line and line.split("=", 1)[1].strip()
                    else line.split("=", 1)[0] + "=" + new
                )
                break
        else:
            at = next((i + 1 for i, line in enumerate(lines) if line == "[General]"), None)
            if at is None:
                lines = ["[General]", f"exclude folders={new}", *lines]
            else:
                lines.insert(at, f"exclude folders={new}")
        baloo_conf().write_text("\n".join(lines) + "\n")


def missing(dirs: list[Path], stop: Path | None = None) -> list[Path]:
    return [d for d in dirs if not excluded(d, stop)]


def hint() -> str:
    if SYSTEM == "Darwin":
        return (
            "Spotlight reads the marker when it indexes a dir. If files already show in "
            "`mdfind -onlyin <dir> -name <file>`, run `sudo mdutil -E /` once, or add the dir "
            "under System Settings > Spotlight > Search Privacy."
        )
    if SYSTEM == "Linux":
        return (
            "tracker3 honors .trackerignore. For GNOME: gsettings set "
            "org.freedesktop.Tracker3.Miner.Files ignored-directories \"[..., '<dir>']\". "
            "Other indexers (recoll, plocate): exclude the dir in their own config."
        )
    return "Windows Search: Settings > Searching Windows > Excluded folders."


def child_ok(parent: Path) -> bool:
    child = parent / "deep" / "child"
    child.mkdir(parents=True, exist_ok=True)
    return SYSTEM == "Windows" or not missing([child], parent.parent)


def self_test() -> int:
    ok = True
    with tempfile.TemporaryDirectory() as t:
        good, fresh = Path(t) / "good", Path(t) / "fresh"
        fresh.mkdir()
        exclude(good)
        for name, cond in (
            ("an excluded dir passes", not missing([good], Path(t))),
            ("a fresh dir fails", missing([fresh], Path(t)) == [fresh]),
            (
                "apply fixes the fresh dir",
                (exclude(fresh), not missing([fresh], Path(t)))[1],
            ),
            ("a child of a marked dir passes", child_ok(good)),
            ("a missing dir fails", missing([Path(t) / "nope"], Path(t)) != []),
        ):
            print(("ok   " if cond else "FAIL ") + name)
            ok = ok and cond
        bench = Path(t) / "bench"
        bench.mkdir()
        env = {
            **os.environ,
            "GITTERRACE_BENCH_DIR": str(bench),
            "CARGO_TARGET_DIR": str(Path(t) / "tgt"),
            "GITTERRACE_INDEX_STOP": t,
        }
        r = subprocess.run(
            [sys.executable, str(ROOT / "scripts/bench/bench-run.py"), "--preflight"],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        refuses = r.returncode != 0 and "not excluded" in r.stderr
        print(("ok   " if refuses else "FAIL ") + "bench preflight refuses an unmarked bench dir")
        exclude(bench)
        exclude(Path(t) / "tgt")
        r = subprocess.run(
            [sys.executable, str(ROOT / "scripts/bench/bench-run.py"), "--preflight"],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
        print(("ok   " if r.returncode == 0 else "FAIL ") + "bench preflight passes once excluded")
        ok = ok and refuses and r.returncode == 0
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--check", action="store_true")
    g.add_argument("--apply", action="store_true")
    g.add_argument("--apply-dir", metavar="PATH")
    g.add_argument("--list", action="store_true")
    g.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    if a.apply_dir:
        exclude(Path(a.apply_dir).expanduser().absolute())
        return 0
    dirs = expected()
    if a.list:
        print("\n".join(map(str, dirs)))
        return 0
    if a.apply:
        for d in dirs:
            exclude(d)
        dirs = missing(dirs)
        if dirs:
            print("could not exclude:\n  " + "\n  ".join(map(str, dirs)), file=sys.stderr)
            return 1
        print(f"search indexing excluded for {len(expected())} dirs")
        print(hint())
        return 0
    bad = missing(dirs)
    if bad:
        print("not excluded from search indexing:\n  " + "\n  ".join(map(str, bad)))
        print("fix: make setup\n" + hint())
        return 1
    print(f"ok: {len(dirs)} dirs excluded from search indexing")
    return 0


if __name__ == "__main__":
    sys.exit(main())
