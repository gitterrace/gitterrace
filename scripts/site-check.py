#!/usr/bin/env python3
"""Website checks (stdlib only): `make site` before a deploy, `make domains` any time.

release: the page tells people to `brew install` from the tap and `cargo install` from the
  repository. Deploying that before both answer publishes an install line that fails, so the
  deploy refuses until the repository, the tap and the `v<version>` release tag exist.
domains: gitterrace.dev serves the page over HTTPS with the generated headers, and every other
  host (www, gitterrace.com, www.gitterrace.com) redirects to it permanently. A www host that
  resolves to nothing looks like DNS propagation and goes unnoticed for weeks; this measures it.

Creates nothing: attaching custom domains and the redirect rule are Cloudflare dashboard
steps, and scripting them needs a Zone:Edit token, too large a credential for this.

  python3 scripts/site-check.py release
  python3 scripts/site-check.py domains
"""

import re
import sys
import tomllib
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CANONICAL = "gitterrace.dev"
REDIRECTS = ("www.gitterrace.dev", "gitterrace.com", "www.gitterrace.com")
TIMEOUT = 15


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


OPENER = urllib.request.build_opener(NoRedirect)


def fetch(url: str, method: str = "GET"):
    """(status, headers) without following redirects; status 0 when nothing answers."""
    req = urllib.request.Request(
        url, method=method, headers={"User-Agent": "gitterrace-site-check"}
    )
    try:
        with OPENER.open(req, timeout=TIMEOUT) as r:
            return r.status, r.headers
    except urllib.error.HTTPError as e:
        return e.code, e.headers
    except (urllib.error.URLError, TimeoutError, OSError):
        return 0, {}


def facts():
    ws = tomllib.loads((ROOT / "Cargo.toml").read_text())
    repo = ws["workspace"]["package"]["repository"].rstrip("/")
    version = ws["workspace"]["package"]["version"]
    tap = ws["workspace"]["metadata"]["dist"]["tap"]
    return repo, version, tap


def release() -> list[str]:
    repo, version, tap = facts()
    m = re.match(r"https://github\.com/([^/]+/[^/]+)$", repo)
    if not m:
        return [f"repository {repo}: not a code-host URL this check knows"]
    host = "https://github.com"
    checks = [
        (f"{repo}", "repository"),
        (f"{host}/{tap}", "Homebrew tap"),
        (f"{repo}/releases/tag/v{version}", f"release tag v{version}"),
    ]
    bad = []
    for url, what in checks:
        status, _ = fetch(url, "HEAD")
        print(
            f"  [{'+' if status == 200 else 'x'}] {what}: {url} ({status or 'no answer'})"
        )
        if status != 200:
            bad.append(f"{what} does not answer at {url}")
    return bad


def domains() -> list[str]:
    bad = []
    status, headers = fetch(f"https://{CANONICAL}/")
    csp = headers.get("Content-Security-Policy", "") if headers else ""
    ok = status == 200 and "script-src 'sha256-" in csp
    print(
        f"  [{'+' if ok else 'x'}] https://{CANONICAL}/ serves the page with the generated CSP ({status or 'no answer'})"
    )
    if not ok:
        bad.append(
            f"https://{CANONICAL}/ answered {status or 'nothing'}, CSP {'present' if csp else 'missing'}"
        )
    status, _ = fetch(f"https://{CANONICAL}/no-such-page")
    print(
        f"  [{'+' if status == 404 else 'x'}] unknown paths answer 404 ({status or 'no answer'})"
    )
    if status != 404:
        bad.append(f"unknown path answered {status or 'nothing'}, want 404")
    for host in REDIRECTS:
        for scheme in ("https", "http"):
            status, headers = fetch(f"{scheme}://{host}/")
            loc = (headers.get("Location", "") if headers else "").rstrip("/")
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
    run = {"release": release, "domains": domains}.get(cmd)
    if not run:
        print(__doc__)
        return 2
    bad = run()
    for b in bad:
        print(f"site-check {cmd}: {b}", file=sys.stderr)
    if not bad:
        print(f"site-check {cmd}: ok")
    return 1 if bad else 0


sys.exit(main())
