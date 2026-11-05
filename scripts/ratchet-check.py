#!/usr/bin/env python3
"""Allowlists and counts only fall (stdlib only). scripts/lint/ratchet.toml declares every
ratcheted allowlist (a file and a dotted path into it) and holds named counts. Against a base
revision, an entry that is new, or a count that is higher, fails. A new guard joins by one
entry in the registry, with no change to this script.

  python3 scripts/ratchet-check.py --base REV    compare the working tree with REV
  python3 scripts/ratchet-check.py --self-test   prove growth goes red on a planted entry
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = "scripts/lint/ratchet.toml"


def parse_registry(text: str) -> dict[str, Any]:
    """The TOML subset the registry uses: [[ratchet]] tables, a [counts] table, scalars."""
    out: dict[str, Any] = {"ratchet": [], "counts": {}}
    cur: dict[str, Any] | None = None
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line == "[[ratchet]]":
            cur = {}
            out["ratchet"].append(cur)
        elif line == "[counts]":
            cur = out["counts"]
        elif cur is None or "=" not in line:
            sys.exit(f"{REGISTRY}: unexpected line: {line}")
        else:
            k, v = (s.strip() for s in line.split("=", 1))
            cur[k] = json.loads(v)
    return out


def dig(cfg: Any, dotted: str) -> Any:
    for part in dotted.split("."):
        cfg = cfg.get(part) if isinstance(cfg, dict) else None
    return cfg


def entries(value: Any) -> dict[str, int]:
    """Flatten an allowlist to {entry: allowed count}: a dict keeps its counts, a list counts 1."""
    if isinstance(value, dict):
        return {str(k): int(v) for k, v in value.items()}
    return {json.dumps(v, sort_keys=True): 1 for v in value or []}


def grown(before: dict[str, int], after: dict[str, int], label: str) -> list[str]:
    bad = []
    for k, n in sorted(after.items()):
        if k not in before:
            bad.append(f"{label}: gained {k}")
        elif n > before[k]:
            bad.append(f"{label}: raised {k} from {before[k]} to {n}")
    return bad


def growth(old: dict[str, Any], new: dict[str, Any]) -> list[str]:
    """Compare two parsed registries plus the files they point at (given as {file: parsed})."""
    bad = grown(entries(old["counts"]), entries(new["counts"]), f"{REGISTRY} [counts]")
    return bad


def at_base(root: Path, base: str, rel: str) -> str | None:
    r = subprocess.run(
        ["git", "-C", str(root), "show", f"{base}:{rel}"],
        capture_output=True,
        text=True,
    )
    return r.stdout if r.returncode == 0 else None


def load(text: str | None, path: str) -> dict[str, int]:
    return entries(dig(json.loads(text), path)) if text else {}


def check(root: Path, base: str) -> list[str]:
    reg = parse_registry((root / REGISTRY).read_text())
    old_text = at_base(root, base, REGISTRY)
    bad = growth(parse_registry(old_text) if old_text else {"counts": {}}, reg)
    for r in reg["ratchet"]:
        label = f"{r['file']} {r['path']}"
        now = (root / r["file"]).read_text()
        before_text = at_base(root, base, r["file"])
        if before_text is None:
            continue
        bad += grown(load(before_text, r["path"]), load(now, r["path"]), label)
    return bad


def self_test() -> None:
    old = {"a": {"x.rs": 2}, "o": {"b": ["p"]}}
    assert grown(load(json.dumps(old), "a"), load(json.dumps(old), "a"), "t") == []
    assert grown({"x": 2}, {"x": 1}, "t") == [], "a lower count passes"
    assert grown({"x": 2}, {}, "t") == [], "a removed entry passes"
    assert grown({"x": 2}, {"x": 3}, "t"), "a raised count fails"
    assert grown({"x": 2}, {"x": 2, "y": 1}, "t"), "a new key fails"
    assert grown(
        load(json.dumps(old), "o.b"),
        load(json.dumps({"o": {"b": ["p", "q"]}}), "o.b"),
        "t",
    )
    reg = parse_registry(
        '[[ratchet]]\nfile = "g.json"\npath = "a"\n[counts]\nsupers = 5\n'
    )
    assert reg["ratchet"] == [{"file": "g.json", "path": "a"}] and reg["counts"] == {
        "supers": 5
    }
    assert growth(reg, {**reg, "counts": {"supers": 6}}), "a raised named count fails"
    assert growth(reg, {**reg, "counts": {"supers": 4}}) == []
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "scripts/lint").mkdir(parents=True)
        (root / "g.json").write_text(json.dumps({"a": {"x.rs": 2}}))
        (root / REGISTRY).write_text(
            '[[ratchet]]\nfile = "g.json"\npath = "a"\n[counts]\n'
        )
        git = [
            "git",
            "-C",
            d,
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.invalid",
        ]
        subprocess.run([*git, "init", "-q", "-b", "main"], check=True)
        subprocess.run([*git, "add", "-A"], check=True)
        subprocess.run([*git, "commit", "-q", "-m", "x"], check=True)
        assert check(root, "HEAD") == [], "a clean tree passes"
        (root / "g.json").write_text(json.dumps({"a": {"x.rs": 9}}))
        assert check(root, "HEAD"), "growing a registered list is red"
        (root / "g.json").write_text(json.dumps({"a": {"x.rs": 1}}))
        assert check(root, "HEAD") == [], "shrinking is green"
    print("ratchet-check self-test: ok")


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    if "--base" not in sys.argv:
        print("ratchet-check: --base REV is required", file=sys.stderr)
        return 2
    bad = check(ROOT, sys.argv[sys.argv.index("--base") + 1])
    for b in bad:
        print(b)
    if bad:
        print(f"ratchet-check: {len(bad)} ratchet(s) grew; fix the offender instead")
        return 1
    print("ratchet-check: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
