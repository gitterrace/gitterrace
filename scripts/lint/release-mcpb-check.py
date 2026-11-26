#!/usr/bin/env python3
"""The MCP registry listing agrees with the release, the dist targets and the bytes.

Tree mode (no argument) checks the templates in scripts/release/mcpb against Cargo.toml:
- server.json validates against the pinned registry schema (vendored beside it, the schema URL
  is the `$id` of that copy), its name is the permanent namespace, its version is the workspace
  version, and it lists exactly one package per dist target, each a `mcpb` package whose URL is
  the tag's release URL for that target and contains "mcp" (the registry's only ownership proof
  for a bundle).
- manifest.json has the pinned manifest_version, and its command, entry point and platform
  overrides name the binary mcpb-pack.py puts in the bundle (override keys are OS names only).
- Cargo.toml wires the two workflows into dist (global and publish slots) and both files exist.

Asset mode (`--assets DIR --tag vX.Y.Z`) checks what a release holds: the rendered server.json
validates, its version is the tag, every package is a dist target with the tag URL, its
fileSha256 is the sha256 of the file in DIR, and each bundle is a zip of exactly manifest.json,
THIRD-PARTY-NOTICES, an optional LICENSE and the one binary the manifest names, with the manifest
version equal to the tag. The bundle job runs it on the files it built, the publish job on the
files it downloaded from the release.

  python3 scripts/lint/release-mcpb-check.py
  python3 scripts/lint/release-mcpb-check.py --assets target/distrib --tag vX.Y.Z
  python3 scripts/lint/release-mcpb-check.py --assets DIR --tag vX.Y.Z --registry
  python3 scripts/lint/release-mcpb-check.py --self-test

With `--registry` the live registry record of that version must list the same packages and
digests as DIR/server.json (the publish job's last step, retried while the record propagates).
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import re
import sys
import tempfile
import time
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path
from types import ModuleType
from typing import Any

import tomllib

ROOT = Path(__file__).resolve().parents[2]
TEMPLATES = "scripts/release/mcpb"
NAMESPACE = "io.github.gitterrace/gitterrace"
MANIFEST_VERSION = "0.3"
OS_NAMES = {"darwin", "win32", "linux"}
SHA = re.compile(r"^[a-f0-9]{64}$")


def load_pack(root: Path) -> ModuleType:
    spec = importlib.util.spec_from_file_location(
        "mcpb_pack", root / "scripts/release/mcpb-pack.py"
    )
    if spec is None or spec.loader is None:
        raise SystemExit("cannot load scripts/release/mcpb-pack.py as a module")
    mod = importlib.util.module_from_spec(spec)
    sys.modules["mcpb_pack"] = mod
    spec.loader.exec_module(mod)
    return mod


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def validate(schema: dict[str, Any], node: Any, value: Any, path: str = "$") -> list[str]:
    """The part of JSON Schema the registry schema uses. An unknown keyword is an error, so a
    schema bump that adds one cannot pass unchecked."""
    known = {
        "$ref",
        "$comment",
        "$id",
        "$schema",
        "definitions",
        "description",
        "example",
        "examples",
        "title",
        "default",
        "type",
        "properties",
        "required",
        "items",
        "enum",
        "const",
        "pattern",
        "minLength",
        "maxLength",
        "format",
        "not",
        "anyOf",
        "allOf",
        "additionalProperties",
    }
    out: list[str] = []
    for key in node:
        if key not in known:
            out.append(f"{path}: schema keyword {key!r} is not supported by this check")
    if "$ref" in node:
        target = schema
        for part in node["$ref"].removeprefix("#/").split("/"):
            target = target[part]
        out += validate(schema, target, value, path)
    types = {
        "object": dict,
        "array": list,
        "string": str,
        "boolean": bool,
        "integer": int,
        "number": (int, float),
    }
    if "type" in node and not isinstance(value, types[node["type"]]):
        return out + [f"{path}: expected {node['type']}"]
    if "const" in node and value != node["const"]:
        out.append(f"{path}: must be {node['const']!r}")
    if "enum" in node and value not in node["enum"]:
        out.append(f"{path}: must be one of {node['enum']}")
    if isinstance(value, str):
        if "pattern" in node and not re.search(node["pattern"], value):
            out.append(f"{path}: {value!r} does not match {node['pattern']}")
        if len(value) < node.get("minLength", 0):
            out.append(f"{path}: shorter than {node['minLength']}")
        if len(value) > node.get("maxLength", 1 << 30):
            out.append(f"{path}: longer than {node['maxLength']}")
    if "not" in node and not validate(schema, node["not"], value, path):
        out.append(f"{path}: {value!r} is not allowed")
    if "anyOf" in node and all(validate(schema, n, value, path) for n in node["anyOf"]):
        out.append(f"{path}: matches none of the alternatives")
    for sub in node.get("allOf", []):
        out += validate(schema, sub, value, path)
    if isinstance(value, dict):
        for name in node.get("required", []):
            if name not in value:
                out.append(f"{path}: missing {name}")
        for name, sub in node.get("properties", {}).items():
            if name in value:
                out += validate(schema, sub, value[name], f"{path}.{name}")
    if isinstance(value, list) and "items" in node:
        for i, item in enumerate(value):
            out += validate(schema, node["items"], item, f"{path}[{i}]")
    return out


def check_server(
    root: Path, doc: dict[str, Any], tag: str, targets: list[str], repo: str
) -> list[str]:
    """Everything about a server.json, template or rendered, that needs no bytes."""
    pack = load_pack(root)
    schema = read_json(root / TEMPLATES / "server.schema.json")
    out = validate(schema, schema["definitions"]["ServerDetail"], doc)
    if doc.get("$schema") != schema["$id"]:
        out.append(
            f"server.json $schema is {doc.get('$schema')}, the vendored schema is {schema['$id']}"
        )
    if doc.get("name") != NAMESPACE:
        out.append(f"server.json name is {doc.get('name')}, the permanent namespace is {NAMESPACE}")
    if doc.get("version") != tag.removeprefix("v"):
        out.append(f"server.json version {doc.get('version')} is not {tag}")
    pkgs = doc.get("packages", [])
    want = {pack.bundle_url(repo, tag, t): t for t in targets}
    got = [p.get("identifier") for p in pkgs]
    for url in sorted(set(want) - set(got)):
        out.append(f"no package for dist target {want[url]} ({url})")
    for url in got:
        if url not in want:
            out.append(f"package {url} is not the tag URL of a dist target")
        if "mcp" not in str(url):
            out.append(f"package URL {url} lacks 'mcp', the registry refuses a bundle without it")
    if len(got) != len(set(got)):
        out.append("a package is listed twice")
    for p in pkgs:
        if p.get("registryType") != "mcpb":
            out.append(f"package {p.get('identifier')} is not registryType mcpb")
        if p.get("transport") != {"type": "stdio"}:
            out.append(f"package {p.get('identifier')} is not stdio")
    return out


def check_manifest(m: dict[str, Any], binary: str, version: str | None) -> list[str]:
    out: list[str] = []
    if m.get("manifest_version") != MANIFEST_VERSION:
        out.append(f"manifest_version is {m.get('manifest_version')}, pinned {MANIFEST_VERSION}")
    if version is not None and m.get("version") != version:
        out.append(f"manifest version {m.get('version')} is not {version}")
    srv = m.get("server", {})
    cfg = srv.get("mcp_config", {})
    if srv.get("type") != "binary":
        out.append("manifest server.type is not binary")
    entry = str(srv.get("entry_point"))
    if entry not in (f"server/{binary}", f"server/{binary}.exe"):
        out.append(f"manifest entry_point {entry} is not server/{binary}")
    if cfg.get("command") != "${__dirname}/" + entry:
        out.append(f"manifest command {cfg.get('command')} is not ${{__dirname}}/{entry}")
    if cfg.get("args") != ["mcp"]:
        out.append('manifest args are not ["mcp"], the subcommand that serves MCP')
    for key, ov in cfg.get("platform_overrides", {}).items():
        if key not in OS_NAMES:
            out.append(f"platform_overrides key {key!r} is not an OS name (no architecture)")
        elif key == "win32" and not str(ov.get("command", "")).endswith(f"server/{binary}.exe"):
            out.append(f"the win32 override does not run server/{binary}.exe")
    return out


def wiring(root: Path) -> list[str]:
    out: list[str] = []
    dist = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))["workspace"][
        "metadata"
    ]["dist"]
    for key, job in (
        ("global-artifacts-jobs", "./release-mcpb"),
        ("publish-jobs", "./publish-mcp-registry"),
    ):
        if job not in dist.get(key, []):
            out.append(f"Cargo.toml [workspace.metadata.dist] {key} lacks {job}")
        wf = root / ".github/workflows" / (job.removeprefix("./") + ".yml")
        if not wf.is_file():
            out.append(f"{wf.relative_to(root)} is missing")
    return out


def tree(root: Path) -> list[str]:
    pack = load_pack(root)
    version, targets, binary, repo = pack.load_cargo(root)
    tag = f"v{version}"
    text = (
        (root / TEMPLATES / "server.json").read_text(encoding="utf-8").replace("@VERSION@", version)
    )
    out = check_server(root, json.loads(text), tag, targets, repo)
    mtext = (root / TEMPLATES / "manifest.json").read_text(encoding="utf-8")
    out += check_manifest(json.loads(mtext.replace("@VERSION@", version)), binary, version)
    return out + wiring(root)


def check_bundle(pack: ModuleType, path: Path, target: str, binary: str, version: str) -> list[str]:
    out: list[str] = []
    exe = f"server/{pack.binary_name(binary, target)}"
    try:
        z = zipfile.ZipFile(path)
    except zipfile.BadZipFile:
        return [f"{path.name} is not a zip"]
    with z:
        names = z.namelist()
        if names != sorted(names):
            out.append(f"{path.name}: entries are not sorted")
        extra = set(names) - {"manifest.json", pack.NOTICES, pack.LICENSE, exe}
        for need in ("manifest.json", pack.NOTICES, exe):
            if need not in names:
                out.append(f"{path.name}: lacks {need}")
        if extra:
            out.append(f"{path.name}: holds more than the binary: {sorted(extra)}")
        if any(i.date_time != pack.MTIME for i in z.infolist()):
            out.append(f"{path.name}: an entry has a time other than the fixed one")
        if "manifest.json" in names:
            m = json.loads(z.read("manifest.json"))
            out += [f"{path.name}: {p}" for p in check_manifest(m, binary, version)]
            if m.get("server", {}).get("entry_point") != exe:
                out.append(f"{path.name}: the manifest does not name {exe}")
            if m.get("compatibility", {}).get("platforms") != [pack.os_of(target)]:
                out.append(f"{path.name}: platforms is not [{pack.os_of(target)!r}]")
    return out


def assets(root: Path, dist: Path, tag: str) -> list[str]:
    pack = load_pack(root)
    version, targets, binary, repo = pack.load_cargo(root)
    if tag != f"v{version}":
        return [f"tag {tag} is not v{version}, the workspace version"]
    sj = dist / "server.json"
    if not sj.is_file():
        return [f"{sj} is missing"]
    doc = json.loads(sj.read_text(encoding="utf-8"))
    out = check_server(root, doc, tag, targets, repo)
    by_target = {pack.bundle_url(repo, tag, t): t for t in targets}
    for p in doc.get("packages", []):
        target = by_target.get(p.get("identifier"))
        if target is None:
            continue
        f = dist / pack.bundle_name(target)
        if not f.is_file():
            out.append(f"{f.name} is not in {dist}")
            continue
        digest = hashlib.sha256(f.read_bytes()).hexdigest()
        if not SHA.match(str(p.get("fileSha256"))):
            out.append(f"{f.name}: fileSha256 is not a lowercase sha256")
        elif p["fileSha256"] != digest:
            out.append(f"{f.name}: fileSha256 {p['fileSha256']} is not the file's {digest}")
        out += check_bundle(pack, f, target, binary, version)
    return out


REGISTRY = "https://registry.modelcontextprotocol.io/v0.1/servers"


def registry_problems(doc: dict[str, Any], record: dict[str, Any]) -> list[str]:
    """The registry record against the server.json that was published."""
    got = {
        p.get("identifier"): p.get("fileSha256")
        for p in record.get("server", {}).get("packages", [])
    }
    want = {p["identifier"]: p["fileSha256"] for p in doc["packages"]}
    out = [f"registry lacks {u}" for u in sorted(set(want) - set(got))]
    out += [f"registry lists {u}, not published" for u in sorted(set(got) - set(want))]
    out += [f"registry digest of {u} differs" for u in want if u in got and got[u] != want[u]]
    return out


def registry(doc: dict[str, Any]) -> list[str]:
    name = urllib.parse.quote(doc["name"], safe="")
    url = f"{REGISTRY}/{name}/versions/{urllib.parse.quote(doc['version'], safe='')}"
    last = "no answer"
    for _ in range(12):
        try:
            with urllib.request.urlopen(url, timeout=20) as r:
                return registry_problems(doc, json.loads(r.read()))
        except OSError as e:
            last = str(e)
        time.sleep(10)
    return [f"the registry has no record at {url}: {last}"]


def fixture(root: Path, dist: Path) -> str:
    """A dist folder of fake archives for every target, packed by the real script."""
    import io
    import tarfile

    pack = load_pack(root)
    version, targets, binary, _ = pack.load_cargo(root)
    for t in targets:
        files = {
            pack.binary_name(binary, t): b"bin " + t.encode(),
            pack.NOTICES: b"notices",
            pack.LICENSE: b"licence",
            "README.md": b"x",
        }
        stem = f"gitterrace-{t}"
        if pack.os_of(t) == "win32":
            with zipfile.ZipFile(dist / f"{stem}.zip", "w") as z:
                for n, d in files.items():
                    z.writestr(f"{stem}/{n}", d)
        else:
            with tarfile.open(dist / f"{stem}.tar.gz", "w:gz") as tf:
                for n, d in files.items():
                    info = tarfile.TarInfo(f"{stem}/{n}")
                    info.size = len(d)
                    tf.addfile(info, io.BytesIO(d))
    pack.pack(root, dist, f"v{version}")
    return f"v{version}"


def self_test() -> int:
    ok = True

    def expect(name: str, found: list[str], red: bool) -> None:
        nonlocal ok
        if bool(found) != red:
            ok = False
            print(f"self-test: {name} should be {'red' if red else 'green'}: {found}")

    expect("tree", tree(ROOT), False)
    with tempfile.TemporaryDirectory() as tmp:
        dist = Path(tmp)
        tag = fixture(ROOT, dist)
        again = Path(tmp) / "again"
        again.mkdir()
        fixture(ROOT, again)
        same = all((dist / p.name).read_bytes() == p.read_bytes() for p in again.glob("*.mcpb"))
        expect("deterministic bundles", [] if same else ["bytes differ"], False)
        expect("assets", assets(ROOT, dist, tag), False)
        sj = dist / "server.json"
        good = sj.read_text(encoding="utf-8")
        doc = json.loads(good)
        digit = doc["packages"][0]["fileSha256"]
        flipped = ("0" if digit[0] != "0" else "1") + digit[1:]
        cases = {
            "a flipped sha digit": good.replace(digit, flipped),
            "a dropped target": json.dumps({**doc, "packages": doc["packages"][1:]}),
            "a URL without mcp": good.replace("gitterrace-mcp-", "gitterrace-"),
            "a wrong version": good.replace(f'"version": "{tag[1:]}"', '"version": "9.9.9"'),
        }
        for name, text in cases.items():
            sj.write_text(text, encoding="utf-8")
            expect(name, assets(ROOT, dist, tag), True)
        sj.write_text(good, encoding="utf-8")
        victim = next(dist.glob("*.mcpb"))
        victim.write_bytes(victim.read_bytes() + b"x")
        expect("a changed bundle", assets(ROOT, dist, tag), True)
    rec = {"server": {"packages": doc["packages"][1:]}}
    expect("a registry record missing a package", registry_problems(doc, rec), True)
    rec = {"server": {"packages": doc["packages"]}}
    expect("a matching registry record", registry_problems(doc, rec), False)
    for name, edit in {
        "an arch override key": ("win32", "x86_64-win32"),
        "a dropped manifest arg": ('"args": ["mcp"]', '"args": []'),
    }.items():
        m = (ROOT / TEMPLATES / "manifest.json").read_text(encoding="utf-8").replace(*edit)
        expect(name, check_manifest(json.loads(m), "git-terrace", None), True)
    print("self-test", "ok" if ok else "FAILED")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    ap.add_argument("--assets", type=Path)
    ap.add_argument("--tag")
    ap.add_argument("--registry", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()
    if args.assets:
        if not args.tag:
            ap.error("--assets needs --tag")
        found = assets(ROOT, args.assets, args.tag)
        if args.registry and not found:
            found = registry(json.loads((args.assets / "server.json").read_text(encoding="utf-8")))
    else:
        found = tree(ROOT)
    for line in found:
        print(f"release-mcpb-check: {line}")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
