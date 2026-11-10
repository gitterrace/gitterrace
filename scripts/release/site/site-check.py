#!/usr/bin/env python3
"""Website checks (stdlib only): `make site` before a deploy, `make domains` any time.

release: the page tells people to `brew install` from the tap and `cargo install` from the
  repository. Deploying that before both answer publishes an install line that fails, so the
  deploy refuses until the repository, the tap and the `v<version>` release tag exist.
csp [DIR]: every inline <script> of every page under DIR (default target/site/) has its exact sha256
  in the CSP of DIR/_headers and of any meta CSP, and no hash is listed that no page has. A changed
  script or a stale _headers fails here instead of being blocked silently by the browser.
  `release` runs it too, before a deploy.
domains: gitterrace.dev serves the page over HTTPS with the generated headers, and every other
  host (www, gitterrace.com, www.gitterrace.com) redirects to it permanently. A www host that
  resolves to nothing looks like DNS propagation and goes unnoticed for weeks; this measures it.

Creates nothing: attaching custom domains and the redirect rule are Cloudflare dashboard
steps, and scripting them needs a Zone:Edit token, too large a credential for this.

  python3 scripts/release/site/site-check.py release
  python3 scripts/release/site/site-check.py csp [DIR]
  python3 scripts/release/site/site-check.py domains
"""

from __future__ import annotations

import base64
import email.message
import hashlib
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CANONICAL = "gitterrace.dev"
REDIRECTS = ("www.gitterrace.dev", "gitterrace.com", "www.gitterrace.com")
TIMEOUT = 15


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args: object, **kwargs: object) -> None:
        return None


OPENER = urllib.request.build_opener(NoRedirect)


Answer = tuple[int, email.message.Message, str]


def fetch(url: str, method: str = "GET", body: bool = False) -> Answer:
    """(status, headers, text) without following redirects. Status 0 and empty headers when
    nothing answers. The text is read only with `body`, else it is empty."""
    req = urllib.request.Request(
        url, method=method, headers={"User-Agent": "gitterrace-site-check"}
    )
    try:
        with OPENER.open(req, timeout=TIMEOUT) as r:
            text = r.read().decode("utf-8", "replace") if body else ""
            return r.status, r.headers, text
    except urllib.error.HTTPError as e:
        return e.code, e.headers, ""
    except (urllib.error.URLError, TimeoutError, OSError):
        return 0, email.message.Message(), ""


INLINE_SCRIPT = re.compile(r"<script\b([^>]*)>(.*?)</script\s*>", re.S | re.I)
META_CSP = re.compile(
    r"<meta\s+http-equiv=[\"']content-security-policy[\"']\s+content=[\"']([^\"']*)[\"']",
    re.I,
)


def script_hashes(html: str) -> set[str]:
    """The CSP source of every inline script: 'sha256-<base64>' over its exact text."""
    out = set()
    for attrs, text in INLINE_SCRIPT.findall(html):
        if re.search(r"\bsrc\s*=", attrs, re.I):
            continue
        digest = hashlib.sha256(text.encode("utf-8")).digest()
        out.add(f"'sha256-{base64.b64encode(digest).decode()}'")
    return out


def csp_problems(where: str, csp: str, html: str | list[str]) -> list[str]:
    """What is wrong with `csp` for `html`, one page or every page the policy covers: a
    missing policy, an inline script whose exact hash it lacks, a hash for no script on any
    page, or unsafe-inline."""
    if not csp:
        return [f"{where}: no Content-Security-Policy"]
    m = re.search(r"script-src([^;]*)", csp)
    listed = set(re.findall(r"'sha256-[A-Za-z0-9+/=]+'", m.group(1) if m else ""))
    pages = [html] if isinstance(html, str) else html
    want = set().union(*(script_hashes(p) for p in pages))
    bad = [
        f"{where}: script-src lacks {h}, the inline script changed"
        for h in sorted(want - listed)
    ]
    bad += [
        f"{where}: script-src lists {h}, which no inline script has"
        for h in sorted(listed - want)
    ]
    if "'unsafe-inline'" in csp:
        bad.append(f"{where}: unsafe-inline")
    return bad


def headers_csp(text: str) -> str:
    """The Content-Security-Policy of the `/*` block in a `_headers` file."""
    m = re.search(r"^/\*\s*$((?:\n[ \t]+.*)*)", text, re.M)
    block = m.group(1) if m else ""
    m = re.search(r"^[ \t]+Content-Security-Policy:\s*(.*)$", block, re.M | re.I)
    return m.group(1).strip() if m else ""


def csp(directory: str | None = None) -> list[str]:
    d = Path(directory) if directory else ROOT / "target" / "site"
    # The `/*` policy covers every page, so its hashes answer for the scripts of all of them:
    # a script only the docs pages carry is still one the policy must list.
    pages = sorted(d.rglob("*.html"))
    htmls = [p.read_text() for p in pages]
    bad = csp_problems(
        f"{d}/_headers", headers_csp((d / "_headers").read_text()), htmls
    )
    for p, html in zip(pages, htmls):
        for meta in META_CSP.findall(html):
            bad += csp_problems(f"{p} meta CSP", meta, html)
    n = len(set().union(*(script_hashes(h) for h in htmls)))
    print(
        f"  [{'x' if bad else '+'}] {n} inline script(s) on {len(pages)} pages in {d} match"
    )
    return bad


def facts() -> tuple[str, str, str]:
    # tomllib is 3.11+, and `csp` runs on older ones, so only `release` imports it.
    import tomllib  # pyright: ignore[reportMissingImports]

    ws = tomllib.loads((ROOT / "Cargo.toml").read_text())
    repo = ws["workspace"]["package"]["repository"].rstrip("/")
    version = ws["workspace"]["package"]["version"]
    tap = ws["workspace"]["metadata"]["dist"]["tap"]
    return repo, version, tap


def release() -> list[str]:
    bad = csp()
    repo, version, tap = facts()
    m = re.match(r"https://github\.com/([^/]+/[^/]+)$", repo)
    if not m:
        return bad + [f"repository {repo}: not a code-host URL this check knows"]
    host = "https://github.com"
    checks = [
        (f"{repo}", "repository"),
        (f"{host}/{tap}", "Homebrew tap"),
        (f"{repo}/releases/tag/v{version}", f"release tag v{version}"),
    ]
    for url, what in checks:
        status, _, _ = fetch(url, "HEAD")
        print(
            f"  [{'+' if status == 200 else 'x'}] {what}: {url} ({status or 'no answer'})"
        )
        if status != 200:
            bad.append(f"{what} does not answer at {url}")
    return bad


def domains() -> list[str]:
    bad = []
    status, headers, page = fetch(f"https://{CANONICAL}/", body=True)
    policy = headers.get("Content-Security-Policy", "")
    wrong = csp_problems(f"https://{CANONICAL}/", policy, page) if status == 200 else []
    for meta in META_CSP.findall(page):
        wrong += csp_problems(f"https://{CANONICAL}/ meta CSP", meta, page)
    ok = status == 200 and not wrong
    print(
        f"  [{'+' if ok else 'x'}] https://{CANONICAL}/ serves the page with a CSP holding its inline script hashes ({status or 'no answer'})"
    )
    if status != 200:
        bad.append(f"https://{CANONICAL}/ answered {status or 'nothing'}")
    bad += wrong
    status, _, _ = fetch(f"https://{CANONICAL}/no-such-page")
    print(
        f"  [{'+' if status == 404 else 'x'}] unknown paths answer 404 ({status or 'no answer'})"
    )
    if status != 404:
        bad.append(f"unknown path answered {status or 'nothing'}, want 404")
    for host in REDIRECTS:
        for scheme in ("https", "http"):
            status, headers, _ = fetch(f"{scheme}://{host}/")
            loc = headers.get("Location", "").rstrip("/")
            good = status in (301, 308) and loc == f"https://{CANONICAL}"
            print(
                f"  [{'+' if good else 'x'}] {scheme}://{host}/ -> {loc or '-'} ({status or 'no answer'})"
            )
            if not good:
                bad.append(
                    f"{scheme}://{host}/ should redirect permanently to https://{CANONICAL}/"
                )
    return bad


def main() -> int:
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    run = {"release": release, "domains": domains, "csp": csp}.get(cmd)
    if not run:
        print(__doc__)
        return 2
    bad = run(*sys.argv[2:3])
    for b in bad:
        print(f"site-check {cmd}: {b}", file=sys.stderr)
    if not bad:
        print(f"site-check {cmd}: ok")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
