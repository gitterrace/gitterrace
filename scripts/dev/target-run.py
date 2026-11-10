#!/usr/bin/env python3
"""Prune stale cargo artefacts and old scratch output under target/ (stdlib only).

Run by `make gc` (DAYS=n, DRY=1, BENCH=1), and with --auto at the start of `make check`.

Why: cargo never garbage-collects target/<profile>/deps, so every toolchain or
dependency bump leaves the old units behind and target/ grows without bound.

Signal, the idea of cargo-sweep (no code, no dependency): a unit is LIVE when
any file in its .fingerprint/<name>-<16hex> dir was accessed or modified
within --days. Cargo reads those files on every build that considers the unit,
so a reused but old artefact stays live, where a bare mtime cut would delete
it and force a rebuild. Entries in deps/, build/ and .fingerprint/ whose hash
is not live go. incremental/ has no fingerprint: a crate dir goes when nothing
in it changed within --days. Any other reader of target/ also refreshes atime,
which keeps too much, never too little.

Safety: a profile is pruned only while this script holds its .cargo-lock
(cargo flocks it for the whole build), and nothing in target/ is touched while
pgrep sees cargo or rustc. target/site-shots and target/app-icon are scratch
output of `make site-shots` and `make app-icon`: their top-level entries go
once nothing in them changed within --days. .git/gitterrace/ caches belong to
users' repos and are never touched. The bench clones are only reported.

    target-run.py [--days N] [--dry-run] [--bench]   prune now, print what went; --bench
                                           also removes the benchmark clones
    target-run.py --auto                   at most once a day, silent, never fails
    target-run.py --self-test
"""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from collections.abc import Callable
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
HASH = re.compile(r"^(?:.*-)?([0-9a-f]{16})$")
DAY = 86400
SCRATCH = ("site-shots", "app-icon")


def unit_hash(name: str) -> str | None:
    """The 16-hex hash in `<prefix>-<hash>[.<ext>...]`, as cargo names units."""
    m = HASH.match(name.split(".", 1)[0])
    return m.group(1) if m else None


def last_used(path: Path) -> float:
    """Newest of the dir's mtime and its files' atime and mtime.

    Not the dir's own atime: listing it here refreshes that, and every unit
    would read as live on the next pass.
    """
    try:
        best = path.stat().st_mtime
        children = list(path.iterdir())
    except OSError:
        return time.time()
    for p in children:
        try:
            st = p.stat()
        except OSError:
            continue
        best = max(best, st.st_atime, st.st_mtime)
    return best


def newest_mtime(path: Path) -> float:
    """Newest mtime anywhere in the tree. Errors read as now, so they keep."""
    try:
        best = path.lstat().st_mtime
        if path.is_dir() and not path.is_symlink():
            for root, dirs, files in os.walk(path):
                for n in dirs + files:
                    best = max(best, os.lstat(os.path.join(root, n)).st_mtime)
    except OSError:
        return time.time()
    return best


def size(path: Path) -> int:
    total = 0
    try:
        if not path.is_dir() or path.is_symlink():
            return path.lstat().st_size
        for root, _dirs, files in os.walk(path):
            for n in files:
                try:
                    total += os.lstat(os.path.join(root, n)).st_size
                except OSError:
                    continue
    except OSError:
        pass
    return total


class Tally:
    def __init__(self, dry_run: bool) -> None:
        self.dry_run = dry_run
        self.count = 0
        self.bytes = 0

    def remove(self, path: Path) -> None:
        self.count += 1
        self.bytes += size(path)
        if self.dry_run:
            return
        if path.is_dir() and not path.is_symlink():
            shutil.rmtree(path, ignore_errors=True)
        else:
            with contextlib.suppress(OSError):
                path.unlink()


def human(n: float) -> str:
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{n} B"
        n /= 1024
    return f"{n} B"


def profiles(target: Path) -> list[Path]:
    """Every dir holding a .fingerprint: target/debug, target/<triple>/release, ..."""
    found: list[Path] = []
    for depth in ("*", "*/*"):
        for fp in target.glob(depth + "/.fingerprint"):
            found.append(fp.parent)
    return sorted(found)


def prune_profile(profile: Path, cutoff: float, tally: Tally) -> bool:
    """Prune one profile under its .cargo-lock. Returns False when cargo holds it."""
    try:
        # Held open across the prune, closed in the finally below, so not a with block.
        lock = open(profile / ".cargo-lock", "a+")  # noqa: SIM115
    except OSError:
        return False
    try:
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            return False
        live: set[str] = set()
        stale: list[Path] = []
        for d in (profile / ".fingerprint").iterdir():
            h = unit_hash(d.name)
            if h is None:
                continue
            if last_used(d) >= cutoff:
                live.add(h)
            else:
                stale.append(d)
        for sub in ("deps", "build"):
            base = profile / sub
            if not base.is_dir():
                continue
            for entry in base.iterdir():
                h = unit_hash(entry.name)
                if h is not None and h not in live:
                    tally.remove(entry)
        for d in stale:
            tally.remove(d)
        inc = profile / "incremental"
        if inc.is_dir():
            for d in inc.iterdir():
                if d.is_dir() and newest_mtime(d) < cutoff:
                    tally.remove(d)
        return True
    finally:
        lock.close()


def prune_scratch(target: Path, cutoff: float, tally: Tally) -> None:
    for name in SCRATCH:
        base = target / name
        if not base.is_dir() or base.is_symlink():
            continue
        for entry in base.iterdir():
            if newest_mtime(entry) < cutoff:
                tally.remove(entry)


def cargo_quiet() -> bool:
    """No cargo or rustc process on the machine (pgrep exits 1 on no match)."""
    for name in ("cargo", "rustc"):
        try:
            rc = subprocess.call(
                ["pgrep", "-x", name],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except OSError:
            return False
        if rc != 1:
            return False
    return True


def bench_dir() -> Path:
    return Path(
        os.environ.get("GITTERRACE_BENCH_DIR", Path.home() / ".cache/gitterrace-bench")
    )


def bench_report() -> str | None:
    d = bench_dir()
    if not d.is_dir():
        return None
    return f"bench clones: {d} holds {human(size(d))}, kept (remove with `make gc BENCH=1`)"


def bench_remove(d: Path, dry_run: bool) -> str:
    """--bench: the cached benchmark clones go too (DRY=1 only reports them)."""
    if not d.is_dir():
        return f"no benchmark clones at {d}"
    held = human(size(d))
    if dry_run:
        return f"would remove {d} ({held})"
    shutil.rmtree(d)
    return f"removed {d} ({held})"


def run(
    target: Path, days: float, dry_run: bool, quiet: Callable[[], bool] = cargo_quiet
) -> str:
    if not quiet():
        return "SKIP cargo or rustc is running, target/ not pruned"
    cutoff = time.time() - days * DAY
    tally = Tally(dry_run)
    busy: list[str] = []
    for p in profiles(target):
        if not prune_profile(p, cutoff, tally):
            busy.append(str(p.relative_to(target)))
    prune_scratch(target, cutoff, tally)
    verb = "would remove" if dry_run else "removed"
    msg = (
        f"OK {verb} {tally.count} entries, {human(tally.bytes)} (unused for {days:g}d)"
    )
    if busy:
        msg += "\nbusy, left alone: " + " ".join(busy)
    return msg


def auto(
    target: Path, days: float, marker: Path, quiet: Callable[[], bool] = cargo_quiet
) -> str | None:
    """At most once a day, stamped before the run so a crash cannot loop.

    Never in CI: actions/cache restores target/ with archived mtimes and no stamp, so
    every run would prune the cache it just restored and then save nothing new."""
    if os.environ.get("CI"):
        return None
    try:
        if time.time() - marker.stat().st_mtime < DAY:
            return None
    except OSError:
        pass
    if not quiet():
        return None
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.touch()
    return run(target, days, False, quiet=quiet)


def set_age(path: Path, days: float) -> None:
    t = time.time() - days * DAY
    os.utime(path, (t, t))


def self_test() -> int:
    fail: list[str] = []
    yes = lambda: True  # noqa: E731
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        target = root / "target"
        prof = target / "debug"
        old_h, new_h = "a" * 16, "b" * 16
        for sub in ("deps", "build", "incremental", ".fingerprint"):
            (prof / sub).mkdir(parents=True)
        (prof / ".cargo-lock").touch()
        shots = target / "site-shots"

        def seed():
            for h, age in ((old_h, 9), (new_h, 0)):
                fp = prof / ".fingerprint" / f"crate-{h}"
                fp.mkdir(exist_ok=True)
                (fp / "lib-crate.json").write_text("{}")
                set_age(fp / "lib-crate.json", age)
                set_age(fp, age)
                # Both artefacts are old: only the fingerprint tells them apart.
                for f in (f"libcrate-{h}.rlib", f"crate-{h}.x.rcgu.o"):
                    (prof / "deps" / f).write_text("x")
                    set_age(prof / "deps" / f, 9)
                (prof / "build" / f"crate-{h}").mkdir(exist_ok=True)
            (prof / "deps" / "unhashed.d").write_text("x")
            set_age(prof / "deps" / "unhashed.d", 9)
            for name, age in (("old-1abc", 9), ("new-2abc", 0)):
                d = prof / "incremental" / name
                (d / "s-1").mkdir(parents=True, exist_ok=True)
                set_age(d / "s-1", age)
                set_age(d, age)
            shots.mkdir(exist_ok=True)
            for name, age in (("old.png", 9), ("new.png", 0)):
                (shots / name).write_text("x")
                set_age(shots / name, age)

        def deps():
            return sorted(p.name for p in (prof / "deps").iterdir())

        seed()
        run(target, 7, False, quiet=lambda: False)
        if f"libcrate-{old_h}.rlib" not in deps():
            fail.append("pruned while cargo was running")

        with open(prof / ".cargo-lock") as held:
            fcntl.flock(held.fileno(), fcntl.LOCK_EX)
            run(target, 7, False, quiet=yes)
        if f"libcrate-{old_h}.rlib" not in deps():
            fail.append("pruned while another process held .cargo-lock")

        out = run(target, 7, True, quiet=yes)
        if f"libcrate-{old_h}.rlib" not in deps() or "would remove" not in out:
            fail.append("--dry-run deleted or did not report")

        run(target, 7, False, quiet=yes)
        got = deps()
        want = sorted(
            ["unhashed.d", f"libcrate-{new_h}.rlib", f"crate-{new_h}.x.rcgu.o"]
        )
        if got != want:
            fail.append(
                f"deps after prune {got}, want {want} (an old artefact of a live unit must stay)"
            )
        if (prof / "build" / f"crate-{old_h}").exists() or not (
            prof / "build" / f"crate-{new_h}"
        ).exists():
            fail.append("build/ not pruned by fingerprint hash")
        if (prof / ".fingerprint" / f"crate-{old_h}").exists():
            fail.append("stale .fingerprint dir kept")
        inc = sorted(p.name for p in (prof / "incremental").iterdir())
        if inc != ["new-2abc"]:
            fail.append(f"incremental after prune {inc}, want ['new-2abc']")
        left = sorted(p.name for p in shots.iterdir())
        if left != ["new.png"]:
            fail.append(f"site-shots after prune {left}, want ['new.png']")

        marker = root / "stamp"
        saved_ci = os.environ.pop("CI", None)
        seed()
        os.environ["CI"] = "1"
        out = auto(target, 7, root / "ci-stamp", yes)
        if out is not None or f"libcrate-{old_h}.rlib" not in deps():
            fail.append("--auto pruned the restored cache under CI=1")
        if (root / "ci-stamp").exists():
            fail.append("--auto stamped under CI=1")
        del os.environ["CI"]
        if auto(target, 7, marker, yes) is None or f"libcrate-{old_h}.rlib" in deps():
            fail.append("--auto did not prune on its first run of the day")
        seed()
        if auto(target, 7, marker, yes) is not None:
            fail.append("--auto ran twice within a day")
        if saved_ci is not None:
            os.environ["CI"] = saved_ci

        clones = root / "bench"
        (clones / "git").mkdir(parents=True)
        if "would remove" not in bench_remove(clones, True) or not clones.is_dir():
            fail.append("--bench --dry-run removed the clones or did not report")
        if "removed" not in bench_remove(clones, False) or clones.exists():
            fail.append("--bench kept the clones")
        if "no benchmark clones" not in bench_remove(clones, False):
            fail.append("--bench without clones did not say so")

    for f in fail:
        print("FAIL:", f)
    if not fail:
        print("target-run self-test OK")
    return 1 if fail else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    ap.add_argument("--days", type=float, default=7)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--auto", action="store_true")
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    a = ap.parse_args()
    if a.self_test:
        return self_test()
    target = REPO / "target"
    if a.auto:
        # --auto must never fail a gate, whatever goes wrong.
        with contextlib.suppress(Exception):
            auto(target, a.days, target / ".gc-stamp")
        return 0
    print(run(target, a.days, a.dry_run))
    if a.bench:
        print(bench_remove(bench_dir(), a.dry_run))
        return 0
    line = bench_report()
    if line:
        print(line)
    return 0


if __name__ == "__main__":
    sys.exit(main())
