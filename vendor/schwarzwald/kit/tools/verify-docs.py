#!/usr/bin/env python3
"""Print the "check your download" section of a getting-started page, or check a page against it.

Reads schwarzwald/config/verify.toml of the current directory (or CONFIG):

    repo = "owner/name"
    methods = ["gh-attestation", "sha256"]   # also "sigstore-bundle", "sbom"
    artifact = "app-1.0.0-linux.tar.gz"      # an example download
    identity = "https://github.com/owner/name/.*"   # sigstore-bundle only
    issuer = "https://token.actions.githubusercontent.com"   # sigstore-bundle only
    sbom = "app-1.0.0-linux.tar.gz.cdx.json" # sbom only

    verify-docs.py [--config FILE]            print the section, markers included
    verify-docs.py [--config FILE] --check PAGE   exit 1 when PAGE differs between the markers
"""

from __future__ import annotations

import argparse
import sys
import textwrap
from pathlib import Path

import tomllib

sys.dont_write_bytecode = True
OPEN = "<!-- schwarzwald:verify -->"
CLOSE = "<!-- /schwarzwald:verify -->"
WIDTH = 80
METHODS = ("gh-attestation", "sha256", "sigstore-bundle", "sbom")


def fence(*lines: str) -> str:
    return "```sh\n" + "\n".join(lines) + "\n```\n"


def prose(text: str) -> str:
    """Wrap a paragraph at WIDTH so the page passes markdownlint line-length."""
    return textwrap.fill(text, WIDTH, break_long_words=False, break_on_hyphens=False) + "\n"


def need(cfg: dict, *keys: str) -> None:
    for k in keys:
        if not cfg.get(k):
            raise ValueError(f"verify.toml: missing {k!r}")


def render(cfg: dict) -> str:
    methods = cfg.get("methods") or []
    if not methods:
        raise ValueError("verify.toml: methods is empty")
    for m in methods:
        if m not in METHODS:
            raise ValueError(f"verify.toml: unknown method {m!r}")
    need(cfg, "artifact")
    f = cfg["artifact"]
    out = ["## Check your download\n"]
    for m in methods:
        if m == "gh-attestation":
            need(cfg, "repo")
            out += ["### Build attestation\n",
                    prose("This proves the file was built by the workflow of that repository, from the commit it names."),
                    fence(f"gh attestation verify {f} --repo {cfg['repo']}")]
        elif m == "sha256":
            out += ["### SHA-256\n",
                    prose("This proves the file arrived whole. Put the file and its `.sha256` in one folder."),
                    "On macOS:\n", fence(f"shasum -a 256 -c {f}.sha256"),
                    prose("If shasum also warns about an improperly formatted line, the check still passed when it prints OK."),
                    "On Linux:\n", fence(f"sha256sum -c {f}.sha256")]
        elif m == "sigstore-bundle":
            need(cfg, "identity", "issuer")
            out += ["### Sigstore bundle\n",
                    prose("This proves the file was signed by the release workflow. Download the file and its `.sigstore.json`."),
                    fence(f"cosign verify-blob --bundle {f}.sigstore.json \\",
                          f"  --certificate-identity-regexp '{cfg['identity']}' \\",
                          f"  --certificate-oidc-issuer {cfg['issuer']} \\",
                          f"  {f}")]
        elif m == "sbom":
            need(cfg, "sbom")
            out += ["### SBOM\n",
                    prose(f"Each download has a CycloneDX SBOM next to it, named like `{cfg['sbom']}`.")]
    return OPEN + "\n\n" + "\n".join(out).rstrip("\n") + "\n\n" + CLOSE + "\n"


def check(page: Path, want: str) -> str | None:
    text = page.read_text(encoding="utf-8")
    a, b = text.find(OPEN), text.find(CLOSE)
    if a < 0 or b < a:
        return f"{page}: missing {OPEN} ... {CLOSE} markers"
    have = text[a : b + len(CLOSE)] + "\n"
    if have != want:
        return f"{page}: verify section differs from verify-docs.py output, regenerate it"
    return None


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--config", default="schwarzwald/config/verify.toml")
    p.add_argument("--check", metavar="PAGE")
    a = p.parse_args()
    try:
        want = render(tomllib.loads(Path(a.config).read_text(encoding="utf-8")))
    except (OSError, ValueError, tomllib.TOMLDecodeError) as e:
        print(e, file=sys.stderr)
        return 1
    if not a.check:
        sys.stdout.write(want)
        return 0
    err = check(Path(a.check), want)
    if err:
        print(err, file=sys.stderr)
    return 1 if err else 0


if __name__ == "__main__":
    sys.exit(main())
