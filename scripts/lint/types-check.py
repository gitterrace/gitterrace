#!/usr/bin/env python3
"""Shared-types guard (stdlib only), run by `make lines`. Its settings live in
scripts/guards/shared-types.json, so another project reuses the script with its own file.

1. No crate defines a type whose name the shared-types crate exports, and no crate renames
   a shared type with `pub use shared::X as Y`.
2. The OpenAPI document types every 200 response: each is an object whose properties are all
   non-empty schemas, except the paths the config skips, with the reason.

  python3 scripts/lint/types-check.py [--config FILE] [--root DIR]
  python3 scripts/lint/types-check.py --self-test    prove each rule goes red on a planted offender
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
DEFAULT_CONFIG = HERE.parent / "guards" / "shared-types.json"
DEF = re.compile(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?(?:struct|enum|union)\s+(\w+)", re.M)
MACRO_DEF = r"{m}!\s*\{{(?:[^{{}}]|\{{[^{{}}]*\}})*?\bpub\s+enum\s+(\w+)"


def sources(root: Path, rels: list[str], cfg: dict[str, Any]) -> list[Path]:
    out = []
    for rel in rels:
        for p in sorted((root / rel).rglob("*.rs")):
            s = "/" + p.relative_to(root).as_posix()
            if any(x in s for x in cfg["skip_path_parts"]):
                continue
            if s.endswith(tuple(cfg["skip_path_suffixes"])):
                continue
            out.append(p)
    return out


def exported(root: Path, cfg: dict[str, Any]) -> set[str]:
    names: set[str] = set()
    for p in sources(root, [cfg["types_src"]], {**cfg, "skip_path_parts": []}):
        text = p.read_text(encoding="utf-8")
        names.update(DEF.findall(text))
        for m in cfg["defining_macros"]:
            names.update(re.findall(MACRO_DEF.format(m=re.escape(m)), text))
    return names


def duplicates(root: Path, cfg: dict[str, Any]) -> list[str]:
    shared = exported(root, cfg)
    allowed = {(a["file"], a["name"]) for a in cfg["allowed_definitions"]}
    types_dir = root / cfg["types_src"]
    crate_name = types_dir.parent.name.replace("-", "_")
    alias = re.compile(
        rf"\b{crate_name}::(?:\{{[^}}]*\}}|\w+)\s+as\s+\w+|\b{crate_name}::\w+\s+as\b"
    )
    problems = []
    for p in sources(root, cfg["scan_roots"], cfg):
        if types_dir in p.parents:
            continue
        rel = p.relative_to(root).as_posix()
        text = p.read_text(encoding="utf-8")
        for name in DEF.findall(text):
            if name in shared and (rel, name) not in allowed:
                problems.append(f"{rel}: defines {name}, which {cfg['types_src']} already exports")
        for m in cfg["defining_macros"]:
            for name in re.findall(MACRO_DEF.format(m=re.escape(m)), text):
                if name in shared and (rel, name) not in allowed:
                    problems.append(
                        f"{rel}: defines {name}, which {cfg['types_src']} already exports"
                    )
        if alias.search(text):
            problems.append(f"{rel}: renames a shared type with `as`, which hides where it lives")
    return problems


def untyped(schema: object, doc: dict[str, Any], depth: int = 0) -> str | None:
    """Why `schema` is not a typed object, or None."""
    if not isinstance(schema, dict) or depth > 8:
        return "not a schema"
    if "$ref" in schema:
        name = schema["$ref"].rsplit("/", 1)[-1]
        target = doc.get("components", {}).get("schemas", {}).get(name)
        return None if target is None and name == "Error" else untyped(target, doc, depth + 1)
    if schema.get("type") != "object":
        return "not an object"
    props = schema.get("properties") or {}
    if not props:
        return "no properties"
    empty = sorted(k for k, v in props.items() if not v)
    return f"empty property schema: {', '.join(empty)}" if empty else None


def openapi(root: Path, cfg: dict[str, Any]) -> list[str]:
    spec = cfg["openapi"]
    if not spec["enabled"]:
        return []
    doc = json.loads((root / spec["file"]).read_text(encoding="utf-8"))
    problems = []
    for path, item in doc.get("paths", {}).items():
        if path.startswith(tuple(spec["skip_path_prefixes"])):
            continue
        for verb, op in item.items():
            ok = op.get("responses", {}).get("200", {})
            schema = ok.get("content", {}).get("application/json", {}).get("schema")
            if schema is None:
                continue
            why = untyped(schema, doc)
            if why:
                problems.append(f"{spec['file']}: {verb.upper()} {path} 200 response: {why}")
    return problems


def problems_in(root: Path, cfg: dict[str, Any]) -> list[str]:
    return duplicates(root, cfg) + openapi(root, cfg)


def self_test() -> int:
    cfg = json.loads(DEFAULT_CONFIG.read_text(encoding="utf-8"))
    cfg["openapi"]["enabled"] = True
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        types = root / cfg["types_src"]
        types.mkdir(parents=True)
        (types / "lib.rs").write_text(
            'string_enum! {\n    pub enum Period {\n        All => "all",\n    }\n}\n'
            "pub struct Person {\n    pub name: String,\n}\n"
        )
        src = root / "crates" / "fixture" / "src"
        src.mkdir(parents=True)
        (root / "docs").mkdir()
        good_api = {
            "paths": {
                "/a": {
                    "get": {
                        "responses": {
                            "200": {
                                "content": {
                                    "application/json": {
                                        "schema": {
                                            "type": "object",
                                            "properties": {"x": {"type": "string"}},
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        skipped = {"paths": {"/skip/x": good_api["paths"]["/a"]}}
        skipped["paths"]["/skip/x"] = {
            "get": {
                "responses": {
                    "200": {
                        "content": {
                            "application/json": {
                                "schema": {"type": "object", "properties": {"x": {}}}
                            }
                        }
                    }
                }
            }
        }
        cfg["openapi"]["skip_path_prefixes"] = ["/skip/"]
        cases = [
            ("a second struct", "pub struct Person {}\n", good_api, True),
            (
                "a second macro enum",
                'string_enum! {\n pub enum Period {\n A => "a",\n }\n}\n',
                good_api,
                True,
            ),
            ("a rename alias", "use gitterrace_types::Person as P;\n", good_api, True),
            ("an unrelated type", "pub struct Other {}\n", good_api, False),
            ("an untyped skipped path", "pub struct Other {}\n", skipped, False),
            (
                "an untyped response",
                "pub struct Other {}\n",
                {
                    "paths": {
                        "/a": {
                            "get": {
                                "responses": {
                                    "200": {
                                        "content": {
                                            "application/json": {
                                                "schema": {
                                                    "type": "object",
                                                    "properties": {"x": {}},
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                True,
            ),
        ]
        for label, code, spec, red in cases:
            (src / "lib.rs").write_text(code)
            (root / "docs/openapi.json").write_text(json.dumps(spec))
            got = bool(problems_in(root, cfg))
            if got != red:
                print(f"self-test FAILED: {label} expected {'red' if red else 'green'}")
                return 1
    print("self-test ok")
    return 0


def main() -> int:
    args = sys.argv[1:]
    if "--self-test" in args:
        return self_test()
    cfg_path = Path(args[args.index("--config") + 1]) if "--config" in args else DEFAULT_CONFIG
    root = Path(args[args.index("--root") + 1]) if "--root" in args else HERE.parent.parent
    cfg = json.loads(cfg_path.read_text(encoding="utf-8"))
    bad = problems_in(root, cfg)
    print("\n".join(bad) or "types: ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
