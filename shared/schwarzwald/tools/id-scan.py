#!/usr/bin/env python3
"""Find real tracker ids and own shas in every blob and message of a history, fast.

Walks each unique blob of REV in REPO once (git rev-list --objects), tokenises six-hex words and
checks them against a set, so thousands of ids cost no more than a few. Ids are the bare
six-hex lines of REPO's `.personal-denylist`, or of IDS_FILE when given. Own shas are REPO's
commits plus the commits of lint.toml `[history] bundles`, minus `[history] keep`. Paths that
match `[history] skip` are not read, and golden digests (`[scope] golden`) only by their `#`
note lines. Prints path:line per hit, the count on stderr. Exit 1 when anything hits.

    id-scan.py REPO REV [IDS_FILE]
"""

from __future__ import annotations

import importlib.util
import re
import subprocess
import sys
from pathlib import Path

sys.dont_write_bytecode = True
LINT = Path(__file__).resolve().parents[1] / "lint"
sys.path.insert(0, str(LINT / "lib"))
import config  # noqa: E402

TOKEN = re.compile(r"\b[0-9a-f]{6}\b")
SHA = re.compile(r"\b[0-9a-f]{7,40}\b")


def own_shas(repo: Path, hist: dict) -> set[str]:
    spec = importlib.util.spec_from_file_location("ownshas", LINT / "own-shas.py")
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader
    spec.loader.exec_module(mod)
    return set(mod.git(repo, "rev-list", "--all").split()) | set(mod.bundle_commits(hist["bundles"]))


def scan(repo: Path, rev: str, ids: set[str]) -> set[str]:
    cfg = config.load(repo)
    skip = [re.compile(s) for s in cfg["history"]["skip"]]
    keep = set(cfg["history"]["keep"])
    own = own_shas(repo, cfg["history"])
    by7: dict[str, list[str]] = {}
    for s in own:
        by7.setdefault(s[:7], []).append(s)

    def is_own(t: str) -> bool:
        return t not in keep and any(s.startswith(t) for s in by7.get(t[:7], []))

    def run(*args: str, inp: str | None = None) -> str:
        return subprocess.run(["git", "-C", str(repo), *args], input=inp, capture_output=True, text=True,
                              check=True).stdout

    blobs: dict[str, str] = {}
    for line in run("rev-list", "--objects", rev).splitlines():
        parts = line.split(" ", 1)
        if len(parts) == 2 and not any(s.search(parts[1]) for s in skip):
            blobs.setdefault(parts[0], parts[1])
    kinds = run("cat-file", "--batch-check=%(objectname) %(objecttype)", inp="\n".join(blobs)).splitlines()
    hits: set[str] = set()
    proc = subprocess.Popen(["git", "-C", str(repo), "cat-file", "--batch"], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE)
    assert proc.stdin and proc.stdout
    for b in (k.split()[0] for k in kinds if k.endswith(" blob")):
        proc.stdin.write((b + "\n").encode())
        proc.stdin.flush()
        size = int(proc.stdout.readline().split()[2])
        data = proc.stdout.read(size)
        proc.stdout.read(1)
        try:
            text = data.decode()
        except UnicodeDecodeError:
            continue
        golden = config.matches(blobs[b], cfg["scope"]["golden"]) and not blobs[b].endswith(".md")
        for n, ln in enumerate(text.splitlines(), 1):
            if golden and not ln.startswith("#"):
                continue  # golden bodies are digests
            if any(t in ids for t in TOKEN.findall(ln)):
                hits.add(f"{blobs[b]}:{n}: {ln.strip()[:120]}")
            for s in SHA.findall(ln):
                if is_own(s):
                    hits.add(f"{blobs[b]}:{n}: own sha {s}: {ln.strip()[:100]}")
    proc.stdin.close()
    proc.wait()
    for m in run("log", "--format=%h%x00%B%x01", rev).split("\x01"):
        if "\x00" not in m:
            continue
        sha, body = m.strip().split("\x00", 1)
        hits |= {f"MESSAGE {sha}: {t}" for t in TOKEN.findall(body) if t in ids}
        hits |= {f"MESSAGE {sha}: own sha {s}" for s in SHA.findall(body) if is_own(s)}
    return hits


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    repo, rev = Path(argv[0]).resolve(), argv[1]
    if len(argv) == 3:
        ids = {x.strip() for x in Path(argv[2]).read_text().replace("|", "\n").split() if x.strip()}
    else:
        ids = config.denylist(repo)[1]
    hits = scan(repo, rev, ids)
    for h in sorted(hits):
        print(h)
    print(f"{len(hits)} hits", file=sys.stderr)
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
