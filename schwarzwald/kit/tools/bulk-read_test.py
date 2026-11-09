#!/usr/bin/env python3
"""bulk-read on a stub endpoint, no network: each refusal holds and a request carries its input.

    bulk-read_test.py   exit 0 when every case holds, else one line per broken case
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

sys.dont_write_bytecode = True
TOOL = Path(os.environ.get("BULK_READ_TOOL") or Path(__file__).resolve().parent / "bulk-read.py")
SEEN: list[dict] = []


class Stub(BaseHTTPRequestHandler):
    def do_POST(self) -> None:
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        SEEN.append(dict(body, _key=self.headers.get("x-api-key")))
        out = json.dumps({"content": [{"type": "text", "text": "- a.txt:2 found it"}]}).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def log_message(self, *a: object) -> None:
        pass


def run(cwd: Path, env_extra: dict[str, str], *args: str) -> subprocess.CompletedProcess[str]:
    env = {k: v for k, v in os.environ.items()
           if k != "ANTHROPIC_API_KEY" and not k.startswith(("JCODE_", "CLAUDE", "BULK_READ_"))}
    env.update(env_extra)
    return subprocess.run([sys.executable, str(TOOL), *args], cwd=cwd, env=env, capture_output=True, text=True)


def run_in(cwd: Path, env_extra: dict[str, str], stdin: bytes, *args: str) -> subprocess.CompletedProcess[bytes]:
    env = {k: v for k, v in os.environ.items()
           if k != "ANTHROPIC_API_KEY" and not k.startswith(("JCODE_", "CLAUDE", "BULK_READ_"))}
    env.update(env_extra)
    return subprocess.run([sys.executable, str(TOOL), *args], cwd=cwd, env=env, capture_output=True, input=stdin)


def bypass_cases(root: Path, jenv: dict[str, str], cli: dict[str, str], jargv: Path, sin: Path) -> list[str]:
    """The audit's bypasses (ST-4): each must refuse and send nothing, on both CLI backends."""
    bad: list[str] = []
    (root / ".env").write_text("TOKEN=sk-live-123\n")
    (root / "notes.txt").symlink_to(".env")
    (root / "credentials").write_text("[default]\naws_secret_access_key = x\n")
    (root / ".npmrc").write_text("//r/:_authToken=x\n")
    (root / "vault.kdbx").write_bytes(b"kdbx")
    (root / "utf16.txt").write_bytes("my home is 12 Hidden Lane\n".encode("utf-16"))
    (root / "split.txt").write_text("my home is 12 Hidden\u200b Lane\n")
    (root / "wide.txt").write_text("my home is 12 \uff28idden Lane\n")
    (root / "nul.bin").write_bytes(b"abc\x00def\n")
    gitd = root / "repo/.git"
    gitd.mkdir(parents=True)
    (gitd / "config").write_text("[remote]\nurl = https://x:tok@h/r\n")
    cases = [("a symlink to .env", ["notes.txt"], b""), ("a case-folded .Env", [".Env"], b""),
             ("an AWS credentials file", ["credentials"], b""), ("an .npmrc", [".npmrc"], b""),
             ("a .kdbx vault", ["vault.kdbx"], b""), ("a .git/config", ["repo/.git/config"], b""),
             ("a UTF-16 denylisted file", ["utf16.txt"], b""), ("a zero-width split word", ["split.txt"], b""),
             ("a fullwidth denylisted word", ["wide.txt"], b""), ("a file with NUL bytes", ["nul.bin"], b""),
             ("stdin without BULK_READ_STDIN=1", ["-"], b"hello\n"),
             ("a denylisted stdin even when allowed", ["-"], b"my home is 12 Hidden Lane\n")]
    for label, args, data in cases:
        for env, sent in ((jenv, jargv), (cli, sin)):
            sent.unlink(missing_ok=True)
            extra = {"BULK_READ_STDIN": "1"} if "allowed" in label else {}
            r = run_in(root, env | extra, data, "q", *args)
            if r.returncode == 0 or sent.exists():
                bad.append(f"sent {label} ({'jcode' if env is jenv else 'claude'}): exit {r.returncode}")
    sin.unlink(missing_ok=True)
    r = run_in(root, cli | {"BULK_READ_STDIN": "1"}, b"plain words\n", "q", "-")
    if r.returncode != 0 or not sin.exists() or "plain words" not in sin.read_text():
        bad.append(f"an allowed clean stdin was not sent: exit {r.returncode} {r.stderr[-200:]!r}")
    r = run_in(root, cli | {"BULK_READ_MAX_BYTES": "10"}, b"", "q", "a.txt")
    if r.returncode != 4:
        bad.append(f"an oversize file is not refused by size first: exit {r.returncode}")
    return bad


def main() -> int:
    srv = HTTPServer(("127.0.0.1", 0), Stub)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    ep = f"http://127.0.0.1:{srv.server_port}/v1/messages"
    live = {"ANTHROPIC_API_KEY": "k-test", "BULK_READ_ENDPOINT": ep}
    bad: list[str] = []
    with tempfile.TemporaryDirectory() as d:
        root = Path(d)
        (root / "a.txt").write_text("alpha\nneedle-content-xyz\n")
        (root / "diary.txt").write_text("my home is 12 Hidden Lane\n")
        keep = root.parent / (root.name + "-private")
        keep.mkdir()
        (keep / "denylist").write_text("hidden lane\n")
        (root / "schwarzwald/config").mkdir(parents=True)
        (root / "schwarzwald/config/lint.toml").write_text(f'[denylist]\npath = "{keep}/denylist"\n')
        (root / "id.pem").write_text("x\n")
        empty = root / "emptybin"
        empty.mkdir()
        stubdir = root / "stubbin"
        stubdir.mkdir()
        log = root / "claude.log"
        stub = stubdir / "claude"
        stub.write_text(
            "#!/bin/sh\n"
            'printf "%s\\n" "$@" > "$STUB_LOG.argv"\n'
            'cat > "$STUB_LOG.stdin"\n'
            '[ -n "$STUB_FAIL" ] && { echo boom >&2; exit 7; }\n'
            "echo '- a.txt:2 via cli'\n"
        )
        stub.chmod(0o755)
        cli = {"PATH": f"{stubdir}:/usr/bin:/bin", "STUB_LOG": str(log), "CLAUDECODE": "1"}
        jstub = stubdir / "jcode"
        jstub.write_text(
            "#!/bin/sh\n"
            'printf "%s\\n" "$@" > "$STUB_LOG.jargv"\n'
            "printf '%s\\n' '- a.txt:2 via jcode' '' '[Tokens] upload: 10 download: 2 cache_read: 0 cache_write: 0' ''\n"
        )
        jstub.chmod(0o755)
        jenv = dict(cli, CLAUDECODE="", JCODE_SESSION_ID="s1", JCODE_ACTIVE_PROVIDER="claude")
        nocli = {"PATH": str(empty)}
        r = run(root, dict(nocli, BULK_READ_ENDPOINT=ep), "q", "a.txt")
        if r.returncode != 2 or "ANTHROPIC_API_KEY" not in r.stderr or "claude" not in r.stderr:
            bad.append(f"neither key nor claude: exit {r.returncode}, want 2 naming both")
        argv, sin = Path(f"{log}.argv"), Path(f"{log}.stdin")
        r = run(root, cli, "where is the needle?", "a.txt")
        args = argv.read_text().split() if argv.exists() else []
        if r.returncode != 0 or "-p" not in args or "claude-haiku-4-5-20251001" not in args or "text" not in args:
            bad.append(f"no key, cli argv wrong: exit {r.returncode} {args} {r.stderr.strip()}")
        elif "where is the needle?" not in sin.read_text() or "needle-content-xyz" not in sin.read_text():
            bad.append("no key, cli stdin lacks question or contents")
        elif "via cli" not in r.stdout:
            bad.append("no key, cli output not printed")
        jargv = Path(f"{log}.jargv")
        argv.unlink(missing_ok=True)
        r = run(root, jenv, "where is the needle?", "a.txt")
        ja = jargv.read_text() if jargv.exists() else ""
        if r.returncode != 0 or "via jcode" not in r.stdout or argv.exists():
            bad.append(f"JCODE_SESSION_ID did not select jcode: exit {r.returncode} {r.stderr.strip()}")
        elif not all(x in ja.split("\n") for x in ("run", "--provider", "claude", "--model", "claude-haiku-4-5-20251001", "--tools", "none")) or "needle-content-xyz" not in ja:
            bad.append(f"jcode argv wrong: {ja[:200]!r}")
        if "[Tokens]" in r.stdout:
            bad.append("jcode token footer not stripped")
        if not r.stdout.startswith("summary by claude-haiku-4-5-20251001"):
            bad.append("jcode header wrong")
        jenv_both = dict(jenv, CLAUDECODE="1", ANTHROPIC_API_KEY="k-test", BULK_READ_ENDPOINT=ep)
        jargv.unlink(missing_ok=True)
        r = run(root, jenv_both, "q", "a.txt")
        if not jargv.exists() or argv.exists():
            bad.append("jcode did not win over claude and key")
        argv.unlink(missing_ok=True)
        r = run(root, dict(jenv, CLAUDECODE="1", JCODE_SESSION_ID=""), "q", "a.txt")
        if not argv.exists():
            bad.append("CLAUDECODE=1 did not select claude")
        argv.unlink(missing_ok=True)
        r = run(root, dict(cli, CLAUDECODE="", CLAUDE_CODE_ENTRYPOINT="cli"), "q", "a.txt")
        if not argv.exists():
            bad.append("CLAUDE_CODE_ENTRYPOINT did not select claude")
        n0 = len(SEEN)
        r = run(root, dict(cli, CLAUDECODE="", ANTHROPIC_API_KEY="k-test", BULK_READ_ENDPOINT=ep), "q", "a.txt")
        if len(SEEN) != n0 + 1:
            bad.append("key alone did not select http")
        argv.unlink(missing_ok=True)
        jargv.unlink(missing_ok=True)
        r = run(root, dict(cli, CLAUDECODE=""), "q", "a.txt")
        if r.returncode != 2 or argv.exists() or jargv.exists():
            bad.append(f"no signal: exit {r.returncode}, want 2 and no backend")
        for forced, want in (("jcode", jargv), ("claude", argv)):
            want.unlink(missing_ok=True)
            r = run(root, dict(jenv_both, BULK_READ_BACKEND=forced), "q", "a.txt")
            if not want.exists():
                bad.append(f"BULK_READ_BACKEND={forced} not honoured")
            argv.unlink(missing_ok=True)
            jargv.unlink(missing_ok=True)
        n0 = len(SEEN)
        r = run(root, dict(jenv_both, BULK_READ_BACKEND="http"), "q", "a.txt")
        if len(SEEN) != n0 + 1 or jargv.exists() or argv.exists():
            bad.append("BULK_READ_BACKEND=http not honoured")
        r = run(root, dict(jenv, JCODE_ACTIVE_PROVIDER="openai"), "q", "a.txt")
        if r.returncode != 2 or "BULK_READ_MODEL" not in r.stderr or jargv.exists():
            bad.append(f"non-claude provider without BULK_READ_MODEL: exit {r.returncode}")
        r = run(root, dict(jenv, JCODE_ACTIVE_PROVIDER="openai", BULK_READ_MODEL="gpt-mini"), "q", "a.txt")
        if r.returncode != 0 or not jargv.exists() or "gpt-mini" not in jargv.read_text():
            bad.append("non-claude provider with BULK_READ_MODEL not used")
        for f in (argv, jargv, sin):
            f.unlink(missing_ok=True)
        for f in ("diary.txt", "id.pem"):
            r = run(root, jenv, "q", f)
            if r.returncode == 0 or jargv.exists():
                bad.append(f"jcode path sent a refused file: {f}")
        r = run(root, dict(jenv, BULK_READ_MAX_BYTES="10"), "q", "a.txt")
        if r.returncode == 0 or jargv.exists():
            bad.append("jcode path sent an oversize input")
        n = len(SEEN)
        r = run(root, dict(cli, CLAUDECODE="", ANTHROPIC_API_KEY="k-test", BULK_READ_ENDPOINT=ep), "q", "a.txt")
        if r.returncode != 0 or len(SEEN) != n + 1:
            bad.append("a key did not use HTTP")
        SEEN.clear()
        for f in (argv, sin):
            f.unlink(missing_ok=True)
        for f in ("diary.txt", "id.pem"):
            r = run(root, cli, "q", f)
            if r.returncode == 0 or argv.exists() or sin.exists():
                bad.append(f"cli path sent a refused file: {f}")
        r = run(root, dict(cli, BULK_READ_MAX_BYTES="10"), "q", "a.txt")
        if r.returncode == 0 or argv.exists() or sin.exists():
            bad.append("cli path sent an oversize input")
        r = run(root, dict(cli, STUB_FAIL="1"), "q", "a.txt")
        if r.returncode == 0 or r.returncode == 2 or "summary by" in r.stdout:
            bad.append(f"nonzero claude exit not a failure: exit {r.returncode}")
        r = run(root, live, "q", "diary.txt")
        if r.returncode == 0 or SEEN:
            bad.append("a denylisted file was sent")
        r = run(root, live, "q", "id.pem")
        if r.returncode == 0 or SEEN:
            bad.append("a secret path was sent")
        r = run(root, dict(live, BULK_READ_MAX_BYTES="10"), "q", "a.txt")
        if r.returncode == 0 or SEEN:
            bad.append("an oversize input was sent")
        r = run(root, live, "where is the needle?", "a.txt")
        req = json.dumps(SEEN[-1]) if SEEN else ""
        if r.returncode != 0 or "where is the needle?" not in req or "needle-content-xyz" not in req:
            bad.append(f"request lacks question or contents: exit {r.returncode} {r.stderr.strip()}")
        elif SEEN[-1]["_key"] != "k-test" or SEEN[-1]["model"] != "claude-haiku-4-5-20251001":
            bad.append("request key or default model wrong")
        if not r.stdout.startswith("summary by claude-haiku-4-5-20251001, verify before editing"):
            bad.append(f"output header wrong: {r.stdout[:80]!r}")
        bad += bypass_cases(root, jenv, cli, jargv, sin)
    for b in bad:
        print(f"FAIL {b}")
    print("ok" if not bad else f"{len(bad)} broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
