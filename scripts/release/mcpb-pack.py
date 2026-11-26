#!/usr/bin/env python3
"""Wrap each dist archive into one MCP bundle (.mcpb) and render the registry's server.json.

A bundle is a zip: manifest.json, THIRD-PARTY-NOTICES, the LICENSE when the archive has one,
and server/<binary>, the one binary of its target. The MCPB manifest names operating
systems only, no architecture, so a bundle holds exactly one target and the release carries
one bundle per dist target.

For every target of [workspace.metadata.dist] in Cargo.toml the script reads
`gitterrace-<target>.tar.gz` (`.zip` on Windows) from the dist folder, writes
`gitterrace-mcp-<target>.mcpb` beside it and prints its sha256. It then renders server.json from
the template with the tag's version, every URL and every digest filled in. The output is
deterministic: sorted entries, a fixed mtime, fixed modes. The checks that the result agrees with
the tag, the targets and the bytes are in scripts/lint/release-mcpb-check.py.

  python3 scripts/release/mcpb-pack.py --tag vX.Y.Z [--dist target/distrib]
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import re
import sys
import tarfile
import zipfile
from pathlib import Path
from typing import Any

import tomllib

ROOT = Path(__file__).resolve().parents[2]
TEMPLATES = "scripts/release/mcpb"
# The earliest time a zip can hold, so the bytes depend on the content alone.
MTIME = (1980, 1, 1, 0, 0, 0)
NOTICES = "THIRD-PARTY-NOTICES"
LICENSE = "LICENSE"


def load_cargo(root: Path) -> tuple[str, list[str], str, str]:
    """Version, dist targets, binary name and repository URL, from the Cargo files."""
    ws = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    version = ws["workspace"]["package"]["version"]
    repo = ws["workspace"]["package"]["repository"]
    targets = ws["workspace"]["metadata"]["dist"]["targets"]
    crate = tomllib.loads((root / "crates/gitterrace/Cargo.toml").read_text(encoding="utf-8"))
    return version, list(targets), crate["bin"][0]["name"], repo


def os_of(target: str) -> str:
    """The MCPB platform name (a Node.js process.platform value) of a Rust target triple."""
    if "windows" in target:
        return "win32"
    if "apple-darwin" in target:
        return "darwin"
    return "linux"


def binary_name(binary: str, target: str) -> str:
    return binary + (".exe" if os_of(target) == "win32" else "")


def bundle_name(target: str) -> str:
    return f"gitterrace-mcp-{target}.mcpb"


def bundle_url(repo: str, tag: str, target: str) -> str:
    return f"{repo}/releases/download/{tag}/{bundle_name(target)}"


def archive_members(path: Path) -> dict[str, bytes]:
    """The files of a dist archive by base name (dist nests them in one folder)."""
    out: dict[str, bytes] = {}
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as z:
            for info in z.infolist():
                if not info.is_dir():
                    out[Path(info.filename).name] = z.read(info)
    else:
        with tarfile.open(path, "r:gz") as t:
            for m in t.getmembers():
                f = t.extractfile(m) if m.isfile() else None
                if f is not None:
                    out[Path(m.name).name] = f.read()
    return out


def find_archive(dist: Path, target: str) -> Path:
    for ext in (".tar.gz", ".zip"):
        p = dist / f"gitterrace-{target}{ext}"
        if p.is_file():
            return p
    sys.exit(f"mcpb-pack: no dist archive for {target} in {dist}")


def manifest_for(
    template: dict[str, Any], version: str, target: str, binary: str
) -> dict[str, Any]:
    """The template for one target: its version, its OS and its binary path, no overrides."""
    m = json.loads(json.dumps(template))
    m["version"] = version
    exe = f"server/{binary_name(binary, target)}"
    cfg = m["server"]["mcp_config"]
    m["server"]["entry_point"] = exe
    cfg["command"] = "${__dirname}/" + exe
    cfg.pop("platform_overrides", None)
    m["compatibility"]["platforms"] = [os_of(target)]
    return m


def build_bundle(members: dict[str, bytes], manifest: dict[str, Any], exe: str) -> bytes:
    """The zip bytes: sorted entries, fixed mtime, fixed modes."""
    files = {
        "manifest.json": (json.dumps(manifest, indent=2) + "\n").encode(),
        NOTICES: members[NOTICES],
        exe: members[Path(exe).name],
    }
    if LICENSE in members:
        files[LICENSE] = members[LICENSE]
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name in sorted(files):
            info = zipfile.ZipInfo(name, MTIME)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            info.external_attr = (0o755 if name == exe else 0o644) << 16
            z.writestr(info, files[name])
    return buf.getvalue()


def pack(root: Path, dist: Path, tag: str) -> dict[str, str]:
    """Write every bundle and server.json into dist. Returns the sha256 by target."""
    version, targets, binary, _ = load_cargo(root)
    if tag != f"v{version}":
        sys.exit(f"mcpb-pack: tag {tag} is not v{version}, the workspace version")
    template = json.loads((root / TEMPLATES / "manifest.json").read_text(encoding="utf-8"))
    shas: dict[str, str] = {}
    for target in targets:
        members = archive_members(find_archive(dist, target))
        exe = f"server/{binary_name(binary, target)}"
        for need in (Path(exe).name, NOTICES):
            if need not in members:
                sys.exit(f"mcpb-pack: the {target} archive has no {need}")
        data = build_bundle(members, manifest_for(template, version, target, binary), exe)
        (dist / bundle_name(target)).write_bytes(data)
        shas[target] = hashlib.sha256(data).hexdigest()
        print(f"{shas[target]}  {bundle_name(target)}")
    (dist / "server.json").write_text(render_server(root, tag, shas), encoding="utf-8")
    return shas


def render_server(root: Path, tag: str, shas: dict[str, str]) -> str:
    """The template with the version, the URLs and the digests filled in."""
    version, _, _, repo = load_cargo(root)
    text = (
        (root / TEMPLATES / "server.json").read_text(encoding="utf-8").replace("@VERSION@", version)
    )
    doc = json.loads(text)
    for pkg in doc["packages"]:
        m = re.search(r"gitterrace-mcp-(.+)\.mcpb$", pkg["identifier"])
        target = m.group(1) if m else ""
        if target not in shas:
            sys.exit(f"mcpb-pack: the template names {pkg['identifier']}, not a dist target")
        pkg["identifier"] = bundle_url(repo, tag, target)
        pkg["fileSha256"] = shas[target]
    return json.dumps(doc, indent=2) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    ap.add_argument("--tag", required=True)
    ap.add_argument("--dist", default="target/distrib")
    args = ap.parse_args()
    pack(ROOT, Path(args.dist), args.tag)
    return 0


if __name__ == "__main__":
    sys.exit(main())
