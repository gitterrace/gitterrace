#!/usr/bin/env python3
"""Ask a cheap model a question about many files and print terse bullets.

    bulk-read.py "<question>" <file|glob|->...

Reads logs, build output, long diffs and unfamiliar files without filling the caller's context.
The answer is a summary: verify before editing, and never route review verdicts, concurrency,
security or crypto through it. Refuses (nothing is sent) when:
  exit 2  no usable backend for the caller's environment, or the usage is wrong
  exit 3  a path is a secret (.env, *.pem, *.key, secrets/) or matches the product's
          .personal-denylist (by path or by content)
  exit 4  the input is over BULK_READ_MAX_BYTES (default 400000)

The backend follows the caller's environment, first match wins:
  1. JCODE_SESSION_ID set: `jcode run --provider $JCODE_ACTIVE_PROVIDER --model M --tools none`, the
     prompt as the message argument (jcode run has no stdin mode), the token footer stripped. M is
     BULK_READ_MODEL, else haiku for provider claude; any other provider needs BULK_READ_MODEL (exit 2).
  2. CLAUDECODE=1 or CLAUDE_CODE_ENTRYPOINT set: `claude -p`, env inherited, prompt on stdin.
  3. ANTHROPIC_API_KEY set: the HTTP API.
  4. otherwise exit 2.
BULK_READ_BACKEND=jcode|claude|http forces one. exit 5: backend failure or timeout.

Env: BULK_READ_MODEL (default claude-haiku-4-5-20251001), BULK_READ_ENDPOINT, BULK_READ_MAX_BYTES.
Python 3.11+, stdlib only.
"""

from __future__ import annotations

import glob
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

sys.dont_write_bytecode = True
MODEL = os.environ.get("BULK_READ_MODEL", "claude-haiku-4-5-20251001")
ENDPOINT = os.environ.get("BULK_READ_ENDPOINT", "https://api.anthropic.com/v1/messages")
MAX_BYTES = int(os.environ.get("BULK_READ_MAX_BYTES", "400000"))
CLI_TIMEOUT = int(os.environ.get("BULK_READ_TIMEOUT", "180"))
SECRET_PATH = re.compile(r"(^|/)(secrets/|\.env(\.[^/]*)?$)|\.(key|pem|p12|pfx)$|(^|/)id_(rsa|ed25519|ecdsa)$")
CHEAP = {"claude": "claude-haiku-4-5-20251001"}
FOOTER = re.compile(r"\n*\[Tokens\][^\n]*\s*\Z")
SYSTEM = (
    "You read files for a coding agent. Answer the question tersely as bullets. Start each bullet "
    "with file:line (the lines are numbered 'N: text'). No preamble. Say plainly when the files do "
    "not answer the question. Do not guess."
)


def denylist() -> list[re.Pattern[str]]:
    for d in [Path.cwd(), *Path.cwd().parents]:
        f = d / ".personal-denylist"
        if f.is_file():
            pats = []
            for line in f.read_text(errors="replace").splitlines():
                line = line.strip()
                if line and not line.startswith("#") and not re.fullmatch(r"[0-9a-fA-F]{6}", line):
                    try:
                        pats.append(re.compile(line, re.I))
                    except re.error:
                        pass
            return pats
    return []


def pick_backend() -> str:
    forced = os.environ.get("BULK_READ_BACKEND")
    if forced:
        if forced not in ("jcode", "claude", "http"):
            die(2, f"BULK_READ_BACKEND={forced!r}: want jcode, claude or http")
        return forced
    if os.environ.get("JCODE_SESSION_ID"):
        return "jcode"
    if os.environ.get("CLAUDECODE") == "1" or os.environ.get("CLAUDE_CODE_ENTRYPOINT"):
        return "claude"
    if os.environ.get("ANTHROPIC_API_KEY"):
        return "http"
    die(2, "no backend: run inside a jcode session (JCODE_SESSION_ID) or Claude Code (CLAUDECODE=1), set ANTHROPIC_API_KEY, or force BULK_READ_BACKEND=jcode|claude|http; nothing was sent")
    return ""


def die(code: int, msg: str) -> None:
    print(f"bulk-read: {msg}", file=sys.stderr)
    sys.exit(code)


def main() -> int:
    if len(sys.argv) < 3:
        die(2, 'usage: bulk-read "<question>" <file|glob|->...')
    question, specs = sys.argv[1], sys.argv[2:]
    backend = pick_backend()
    model = MODEL
    if backend == "jcode":
        provider = os.environ.get("JCODE_ACTIVE_PROVIDER", "")
        model = os.environ.get("BULK_READ_MODEL") or CHEAP.get(provider, "")
        if not provider or not model:
            die(2, f"jcode provider {provider!r} has no known cheap model: set BULK_READ_MODEL; nothing was sent")
    need = {"jcode": "jcode", "claude": "claude"}.get(backend)
    if need and not shutil.which(need):
        die(2, f"backend {backend} chosen but no `{need}` binary on PATH; nothing was sent")
    if backend == "http" and not os.environ.get("ANTHROPIC_API_KEY"):
        die(2, "backend http needs ANTHROPIC_API_KEY; nothing was sent")
    deny = denylist()
    parts: list[str] = []
    total = 0
    for spec in specs:
        if spec == "-":
            name, data = "stdin", sys.stdin.read()
            items = [(name, data)]
        else:
            paths = sorted(glob.glob(spec, recursive=True)) or [spec]
            items = []
            for p in paths:
                if not os.path.isfile(p):
                    die(2, f"not a file: {p}")
                if SECRET_PATH.search(p) or any(r.search(p) for r in deny):
                    die(3, f"refused, secret or denylisted path: {p}")
                items.append((p, Path(p).read_text(errors="replace")))
        for name, data in items:
            if any(r.search(data) for r in deny):
                die(3, f"refused, content matches .personal-denylist: {name}")
            numbered = "\n".join(f"{i}: {ln}" for i, ln in enumerate(data.splitlines(), 1))
            parts.append(f"=== {name} ===\n{numbered}\n")
            total += len(numbered.encode())
    if total > MAX_BYTES:
        die(4, f"refused, {total} bytes is over BULK_READ_MAX_BYTES={MAX_BYTES}; narrow the files or grep first")
    if backend in ("jcode", "claude"):
        prompt = f"{SYSTEM}\n\nQuestion: {question}\n\n" + "\n".join(parts)
        if backend == "jcode":
            cmd = ["jcode", "run", "--provider", os.environ["JCODE_ACTIVE_PROVIDER"], "--model", model, "--tools", "none", prompt]
            stdin = None
        else:
            cmd = ["claude", "-p", "--model", model, "--output-format", "text"]
            stdin = prompt
        try:
            p = subprocess.run(cmd, input=stdin, capture_output=True, text=True, timeout=CLI_TIMEOUT)
        except subprocess.TimeoutExpired:
            die(5, f"{backend} timed out after {CLI_TIMEOUT}s")
        except OSError as e:
            die(5, f"{backend} could not run: {e}")
        if p.returncode != 0:
            die(5, f"{backend} failed with exit {p.returncode}: {p.stderr.strip()[:300]}")
        out = FOOTER.sub("", p.stdout).strip()
        if not out:
            die(5, f"{backend} returned no output")
        print(f"summary by {model}, verify before editing\n{out}")
        return 0
    body = {
        "model": MODEL,
        "max_tokens": 2000,
        "temperature": 0,
        "system": SYSTEM,
        "messages": [{"role": "user", "content": f"Question: {question}\n\n" + "\n".join(parts)}],
    }
    req = urllib.request.Request(
        ENDPOINT,
        json.dumps(body).encode(),
        {"content-type": "application/json", "x-api-key": os.environ["ANTHROPIC_API_KEY"], "anthropic-version": "2023-06-01"},
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            out = json.load(r)
    except urllib.error.HTTPError as e:
        die(5, f"API error {e.code}: {e.read().decode(errors='replace')[:300]}")
    except (urllib.error.URLError, TimeoutError) as e:
        die(5, f"API unreachable: {e}")
    text = "".join(b.get("text", "") for b in out.get("content", []) if b.get("type") == "text")
    print(f"summary by {MODEL}, verify before editing\n{text.strip()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
