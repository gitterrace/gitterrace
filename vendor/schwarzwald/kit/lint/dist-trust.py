#!/usr/bin/env python3
"""CONV-DIST-TRUST: a product that ships with cargo-dist signs and describes its releases.

The cargo-dist config must set `cargo-auditable = true`, `cargo-cyclonedx = true`,
`github-attestations = true` and a strong `checksum` (sha256, sha512, sha3-256, sha3-512,
blake2s or blake2b). It is read from `[workspace.metadata.dist]` in Cargo.toml and from
`[dist]` or `[workspace.metadata.dist]` in dist-workspace.toml. A product with no cargo-dist
config is not checked. The snippet to paste is `configs/dist/trust.toml`.

  dist-trust.py [--repo DIR]
  dist-trust.py --self-test
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-DIST-TRUST"
REQUIRED = {"cargo-auditable": True, "cargo-cyclonedx": True, "github-attestations": True}
STRONG = {"sha256", "sha512", "sha3-256", "sha3-512", "blake2s", "blake2b"}
SNIPPET = Path(__file__).resolve().parents[1] / "configs/dist/trust.toml"
REMEDY = "paste the keys from the kit's configs/dist/trust.toml into the dist config"


def dist_tables(name: str, data: dict) -> list[tuple[str, dict]]:
    """Each cargo-dist table in a parsed file, with the label to name it by."""
    found = []
    if name == "dist-workspace.toml" and isinstance(data.get("dist"), dict):
        found.append(("[dist]", data["dist"]))
    meta = data.get("workspace", {}).get("metadata", {}).get("dist")
    if isinstance(meta, dict):
        found.append(("[workspace.metadata.dist]", meta))
    return found


def problems(file: str, label: str, table: dict) -> list[str]:
    out = []
    for key, want in REQUIRED.items():
        if key not in table:
            out.append(f"{file} {label} lacks `{key} = true`; {REMEDY}")
        elif table[key] is not want:
            out.append(f"{file} {label} sets `{key} = {table[key]!r}`, must be true; {REMEDY}")
    if "checksum" not in table:
        out.append(f'{file} {label} lacks `checksum = "sha256"`; {REMEDY}')
    elif table["checksum"] not in STRONG:
        out.append(f'{file} {label} sets `checksum = {table["checksum"]!r}`, must be sha256 or stronger; {REMEDY}')
    return out


def check(root: Path) -> tuple[list[str], int]:
    """The problems found and how many cargo-dist tables were read."""
    bad, seen = [], 0
    for name in ("Cargo.toml", "dist-workspace.toml"):
        path = root / name
        if not path.is_file():
            continue
        try:
            data = tomllib.loads(path.read_text(encoding="utf-8"))
        except tomllib.TOMLDecodeError as e:
            bad.append(f"{name} does not parse: {e}")
            continue
        for label, table in dist_tables(name, data):
            seen += 1
            bad += problems(name, label, table)
    return bad, seen


def self_test() -> bool:
    full = {"cargo-auditable": True, "cargo-cyclonedx": True, "github-attestations": True, "checksum": "sha256"}
    snippet = tomllib.loads(SNIPPET.read_text(encoding="utf-8")) if SNIPPET.exists() else full
    ok = (
        snippet == full
        and not problems("C", "[t]", full)
        and not problems("C", "[t]", full | {"checksum": "sha512"})
        and bool(problems("C", "[t]", full | {"checksum": "sha1"}))
        and bool(problems("C", "[t]", {k: v for k, v in full.items() if k != "checksum"}))
        and bool(problems("C", "[t]", full | {"cargo-cyclonedx": False}))
        and len(problems("C", "[t]", {})) == 4
    )
    if not ok:
        print(f"FAIL [{RULE}] self-test", file=sys.stderr)
    return ok


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        if not self_test():
            return 1
        print("dist-trust self-test ok")
        return 0
    root, _ = config.take_repo(argv)
    bad, seen = check(root)
    for b in bad:
        print(f"FAIL [{RULE}] {b}", file=sys.stderr)
    if not seen and not bad:
        print(f"{RULE}: no cargo-dist config, nothing to check")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
