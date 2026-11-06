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

import subprocess
import sys
import tempfile
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parent.parent
AST = ROOT / "scripts" / "ast-grep"

# Rules known blind inside macro bodies: rule id -> why. Fill only with a reason.
BLIND: dict[str, str] = {}


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
            spec = yaml.safe_load(f.read_text())
            invalid = [wrap(c) for c in spec.get("invalid", [])]
            if not invalid:
                continue
            (t / "tests" / f.name).write_text(
                yaml.safe_dump({"id": spec["id"], "valid": [], "invalid": invalid})
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
    print("self-test", "ok" if red and green else "FAILED")
    return 0 if red and green else 1


if __name__ == "__main__":
    sys.exit(main())
