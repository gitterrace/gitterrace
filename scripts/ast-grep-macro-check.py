#!/usr/bin/env python3
"""Re-runs every invalid example of every ast-grep rule inside a macro call.

A rule that matches only at item level goes blind once the same code sits in a macro body
(`m!{ ... }`), where the parser sees bare tokens. Each invalid case of scripts/ast-grep/tests
is wrapped and run through `ast-grep test`; a rule that misses is reported unless it is on
BLIND with a reason. The list only shrinks.

  python3 scripts/ast-grep-macro-check.py              check the rules
  python3 scripts/ast-grep-macro-check.py --self-test  prove it goes red on an item-only rule
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
AST = ROOT / "scripts" / "ast-grep"

# Rules known blind inside macro bodies: rule id -> why. Fill only with a reason.
BLIND: dict[str, str] = {}


def scalar(text: str) -> str:
    """One quoted YAML scalar: single quotes double a quote, double quotes escape like JSON."""
    if text.startswith("'") and text.endswith("'") and len(text) > 1:
        return text[1:-1].replace("''", "'")
    if text.startswith('"') and text.endswith('"') and len(text) > 1:
        return json.loads(text)
    return text


def read_spec(text: str) -> dict[str, Any]:
    """The rule test files: `id: x` and the lists `valid:` and `invalid:` of quoted scalars or
    `|` blocks. No other YAML is read, so a new shape fails loudly instead of being misread."""
    spec: dict[str, Any] = {"valid": [], "invalid": []}
    key, block = "", None
    for line in text.splitlines():
        if block is not None:
            if line.startswith("    ") or not line.strip():
                block.append(line[4:])
                continue
            spec[key].append("\n".join(block).rstrip("\n") + "\n")
            block = None
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line.startswith(" "):
            name, _, rest = line.partition(":")
            if name == "id":
                spec["id"] = scalar(rest.strip())
            elif name in spec and not rest.strip():
                key = name
            else:
                raise ValueError(f"unsupported line: {line}")
        elif line.startswith("  - "):
            item = line[4:].strip()
            if item == "|":
                block = []
            else:
                spec[key].append(scalar(item))
        else:
            raise ValueError(f"unsupported line: {line}")
    if block is not None:
        spec[key].append("\n".join(block).rstrip("\n") + "\n")
    return spec


def wrap(code: str) -> str:
    return "m!{\n" + code + "\n}"


def blind_rules(rules: Path, tests: Path) -> list[str]:
    """Rule ids with an invalid example that stops matching once wrapped in a macro."""
    found = []
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        (t / "tests").mkdir()
        (t / "sgconfig.yml").write_text(
            f"ruleDirs:\n  - {rules}\ntestConfigs:\n  - testDir: tests\n"
        )
        for f in sorted(tests.glob("*-test.yml")):
            spec = read_spec(f.read_text())
            invalid = [wrap(c) for c in spec.get("invalid", [])]
            if not invalid:
                continue
            (t / "tests" / f.name).write_text(
                json.dumps({"id": spec["id"], "valid": [], "invalid": invalid})
            )
        r = subprocess.run(
            ["ast-grep", "test", "--skip-snapshot-tests"],
            cwd=t,
            capture_output=True,
            text=True,
            check=False,
        )
        for line in r.stdout.splitlines():
            if line.startswith("[Missing] Expect rule "):
                found.append(line.split()[3])
    return sorted(set(found))


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bad = [i for i in blind_rules(AST / "rules", AST / "tests") if i not in BLIND]
    for i in bad:
        print(f"ast-grep rule {i} misses an invalid example inside a macro body")
    stale = [i for i in BLIND if i not in blind_rules(AST / "rules", AST / "tests")]
    for i in stale:
        print(f"{i} is on BLIND but now matches in macros, drop it")
    print("" if bad or stale else "ast-grep-macro: ok")
    return 1 if bad or stale else 0


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        t = Path(tmp)
        (t / "rules").mkdir()
        (t / "tests").mkdir()
        (t / "rules" / "item-only.yml").write_text(
            "id: item-only\nlanguage: rust\nseverity: error\nmessage: m\n"
            "rule:\n  kind: field_declaration\n"
        )
        (t / "tests" / "item-only-test.yml").write_text(
            "id: item-only\nvalid:\n  - 'fn f() {}'\ninvalid:\n  - 'struct S { a: u8 }'\n"
        )
        red = blind_rules(t / "rules", t / "tests") == ["item-only"]
        (t / "rules" / "item-only.yml").write_text(
            "id: item-only\nlanguage: rust\nseverity: error\nmessage: m\n"
            "rule:\n  regex: 'a: u8'\n"
        )
        green = blind_rules(t / "rules", t / "tests") == []
        # A python without PyYAML runs the whole check: a module named yaml that refuses to
        # import stands in for one, and the run must still pass.
        (t / "nolib").mkdir()
        (t / "nolib" / "yaml.py").write_text("raise ImportError('no yaml here')\n")
        env = {**os.environ, "PYTHONPATH": str(t / "nolib")}
        bare = subprocess.run(
            [sys.executable, str(Path(__file__).resolve())],
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
    ok = red and green and bare.returncode == 0
    if bare.returncode:
        print(bare.stdout + bare.stderr)
    print("self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
