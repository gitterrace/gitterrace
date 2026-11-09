#!/usr/bin/env python3
"""Allowlists and counts only fall (stdlib only).

The product's registry, schwarzwald/config/ratchet.toml, declares every ratcheted allowlist (a
JSON file and a dotted path into it) and holds named counts. Against a base revision, an entry
that is new or a count that is higher fails. A ratchet the base registry declares must still be
declared: deleting a [[ratchet]] table would otherwise drop its allowlist from the check
unseen. A ratchet retired on purpose is named in the registry's `retired` list, with the reason
in a comment beside it, so the removal is reviewed like any other change. A new guard joins by
one entry, with no change to this script.

  ratchet-check.py [<repo>] --base REV    compare the working tree of <repo> (default .) with REV
  ratchet-check.py --self-test             prove growth and a dropped entry go red
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

if sys.version_info < (3, 11):
    sys.exit("ratchet-check.py needs python 3.11 or newer (tomllib)")
import tomllib

REGISTRY = "schwarzwald/config/ratchet.toml"


def parse_registry(text: str) -> dict[str, Any]:
    raw = tomllib.loads(text)
    unknown = set(raw) - {"ratchet", "counts", "retired"}
    if unknown:
        sys.exit(f"{REGISTRY}: unknown tables {sorted(unknown)}")
    return {"ratchet": raw.get("ratchet", []), "counts": raw.get("counts", {}),
            "retired": raw.get("retired", {}).get("entries", [])}


def dig(cfg: Any, dotted: str) -> Any:
    for part in dotted.split("."):
        cfg = cfg.get(part) if isinstance(cfg, dict) else None
    return cfg


def entries(value: Any) -> dict[str, int]:
    """An allowlist as {entry: allowed count}: a dict keeps its counts, a list counts 1 each."""
    if isinstance(value, dict):
        return {str(k): int(v) for k, v in value.items()}
    return {json.dumps(v, sort_keys=True): 1 for v in value or []}


def grown(before: dict[str, int], after: dict[str, int], label: str, new_ok: bool = False) -> list[str]:
    """An allowlist may not gain a key. A [counts] entry may join (new_ok), only a raised one fails."""
    bad = []
    for k, n in sorted(after.items()):
        if k not in before:
            if new_ok:
                continue
            bad.append(f"{label}: gained {k}")
        elif n > before[k]:
            bad.append(f"{label}: raised {k} from {before[k]} to {n}")
    return bad


def key(r: dict[str, str]) -> str:
    return f"{r['file']} {r['path']}"


def dropped(old: dict[str, Any], new: dict[str, Any]) -> list[str]:
    """Ratchets of the base registry that the new one no longer declares, unless retired."""
    have = {key(r) for r in new["ratchet"]}
    retired = set(new["retired"])
    return [f"{REGISTRY}: the ratchet {k} was removed; name it in [retired] entries to retire it"
            for k in sorted({key(r) for r in old["ratchet"]} - have - retired)]


def at_base(root: Path, base: str, rel: str) -> str | None:
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    r = subprocess.run(["git", "-C", str(root), "show", f"{base}:{rel}"], capture_output=True, text=True, env=env)
    return r.stdout if r.returncode == 0 else None


def load(text: str | None, path: str) -> dict[str, int]:
    return entries(dig(json.loads(text), path)) if text else {}


def check(root: Path, base: str) -> list[str]:
    reg = parse_registry((root / REGISTRY).read_text())
    old_text = at_base(root, base, REGISTRY)
    old = parse_registry(old_text) if old_text else {"ratchet": [], "counts": {}, "retired": []}
    bad = grown(entries(old["counts"]), entries(reg["counts"]), f"{REGISTRY} [counts]", new_ok=True)
    bad += dropped(old, reg)
    known = {key(r) for r in old["ratchet"]}
    for r in reg["ratchet"]:
        if key(r) not in known:
            continue
        before_text = at_base(root, base, r["file"])
        if before_text is None:
            continue
        bad += grown(load(before_text, r["path"]), load((root / r["file"]).read_text(), r["path"]), key(r))
    return bad


def self_test() -> None:
    old = {"a": {"x.rs": 2}, "o": {"b": ["p"]}}
    assert grown(load(json.dumps(old), "a"), load(json.dumps(old), "a"), "t") == []
    assert grown({"x": 2}, {"x": 1}, "t") == [], "a lower count passes"
    assert grown({"x": 2}, {}, "t") == [], "a removed allowlist entry passes"
    assert grown({"x": 2}, {"x": 3}, "t"), "a raised count fails"
    assert grown({"x": 2}, {"x": 2, "y": 1}, "t"), "a new key fails"
    assert grown({"x": 2}, {"x": 2, "y": 1}, "t", new_ok=True) == [], "a new count joins"
    assert grown({"x": 2}, {"x": 3}, "t", new_ok=True), "a raised count still fails"
    assert grown(load(json.dumps(old), "o.b"), load(json.dumps({"o": {"b": ["p", "q"]}}), "o.b"), "t")
    reg_text = '[[ratchet]]\nfile = "g.json"\npath = "a"\n\n[counts]\nsupers = 5\n'
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "schwarzwald/config").mkdir(parents=True)
        (root / "g.json").write_text(json.dumps({"a": {"x.rs": 2}}))
        (root / REGISTRY).write_text(reg_text)
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        git = ["git", "-C", d, "-c", "user.name=t", "-c", "user.email=t@example.invalid"]
        for args in (["init", "-q", "-b", "main"], ["add", "-A"], ["commit", "-q", "-m", "x"]):
            subprocess.run([*git, *args], check=True, env=env)
        assert check(root, "HEAD") == [], "a clean tree passes"
        (root / "g.json").write_text(json.dumps({"a": {"x.rs": 9}}))
        assert check(root, "HEAD"), "growing a registered list is red"
        (root / "g.json").write_text(json.dumps({"a": {"x.rs": 1}}))
        assert check(root, "HEAD") == [], "shrinking is green"
        (root / REGISTRY).write_text(reg_text.replace("supers = 5", "supers = 6"))
        assert check(root, "HEAD"), "a raised named count is red"
        (root / REGISTRY).write_text(reg_text + "fresh = 72\n")
        assert check(root, "HEAD") == [], "a new named count joins green"
        (root / "h.json").write_text(json.dumps({"a": {"y.rs": 1}}))
        subprocess.run([*git, "add", "h.json"], check=True, env=env)
        subprocess.run([*git, "commit", "-q", "-m", "h"], check=True, env=env)
        (root / "h.json").write_text(json.dumps({"a": {"y.rs": 1, "z.rs": 1}}))
        (root / REGISTRY).write_text(reg_text + '\n[[ratchet]]\nfile = "h.json"\npath = "a"\n')
        assert check(root, "HEAD") == [], "a new [[ratchet]] joins with its list as it stands"
        (root / "g.json").write_text(json.dumps({"a": {"x.rs": 9}}))
        (root / REGISTRY).write_text("[counts]\nsupers = 5\n")
        assert check(root, "HEAD"), "a deleted [[ratchet]] entry is red, its list cannot grow unseen"
        (root / REGISTRY).write_text('[retired]\nentries = ["g.json a"]\n\n[counts]\nsupers = 5\n')
        assert check(root, "HEAD") == [], "a ratchet named in [retired] is a reviewed removal"
    print("ratchet-check self-test: ok")


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        self_test()
        return 0
    if "--base" not in argv:
        print("ratchet-check: --base REV is required", file=sys.stderr)
        return 2
    i = argv.index("--base")
    base, rest = argv[i + 1], argv[:i] + argv[i + 2:]
    root = Path(rest[0] if rest else ".").resolve()
    bad = check(root, base)
    for b in bad:
        print(b)
    if bad:
        print(f"ratchet-check: {len(bad)} ratchet(s) grew or went missing; fix the offender instead")
        return 1
    print("ratchet-check: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
