#!/usr/bin/env python3
"""Website checks (stdlib only): `make site` before a deploy, `make domains` any time.

release: the page tells people to `brew install` from the tap and `cargo install` from the
  repository. Deploying that before both answer publishes an install line that fails, so the
  deploy refuses until the repository, the tap and the `v<version>` release tag exist, and
  until that release's shell installer holds a digest to check each download against.
csp [DIR]: every inline <script> of every page under DIR has its exact sha256 in the CSP of
  DIR/_headers and of any meta CSP, and no hash is listed that no page has. A changed script or
  a stale _headers fails here instead of being blocked silently by the browser. Without DIR it
  builds the site into target/site first (`cargo xtask site`, no tracked file is written).
  `release` runs it too, before a deploy.
links [DIR]: every link of a docs page takes the one shared link color (the `a` rule of
  DIR/assets/site.css), and only the chrome (bar, lists, pager, footer) may set its own. The
  footer of a docs page is a landmark with a labelled nav and holds the Edit link last, pushed
  to the right on a wide screen and stacked on a narrow one. Without DIR it reads target/site.
domains: gitterrace.dev serves the page over HTTPS with the generated headers, and every other
  host (www, gitterrace.com, www.gitterrace.com) redirects to it permanently. A www host that
  resolves to nothing looks like DNS propagation and goes unnoticed for weeks; this measures it.

Creates nothing: attaching custom domains and the redirect rule are Cloudflare dashboard
steps, and scripting them needs a Zone:Edit token, too large a credential for this.

  python3 scripts/release/site/site-check.py release
  python3 scripts/release/site/site-check.py csp [DIR]
  python3 scripts/release/site/site-check.py links [DIR]
  python3 scripts/release/site/site-check.py domains
  python3 scripts/release/site/site-check.py --self-test
"""

from __future__ import annotations

import base64
import email.message
import hashlib
import re
import subprocess
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
# The shell installer dist attaches to each release, and the line in it that holds a digest.
INSTALLER = "gitterrace-installer.sh"
CHECKSUM = "_checksum_value="


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
    if directory:
        d = Path(directory)
    else:
        # The site is build output, so a fresh clone has none: build it from the tree as it is.
        # `xtask site` writes only target/site, never a tracked file. A failed build fails here.
        subprocess.run(
            ["cargo", "xtask", "site"], cwd=ROOT, check=True, stdout=subprocess.DEVNULL
        )
        d = ROOT / "target" / "site"
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


# Selectors that style a link of the page chrome, not of the text: only these may set their own
# link color. A content link such as `.groups li a` must take the shared `a` rule instead.
CHROME = (".top", ".docnav", ".outline", ".menu", ".pager", "footer", ".skip", ".brand")
RULE = re.compile(r"([^{}]+)\{([^{}]*)\}")


def css_problems(css: str) -> list[str]:
    css = re.sub(r"/\*.*?\*/", "", css, flags=re.S)
    rules = [(re.sub(r"\s+", " ", a).strip(), b) for a, b in RULE.findall(css)]
    bad = []
    if not any(
        sel == "a" and "color:var(--accent-ink)" in body for sel, body in rules
    ):
        bad.append("site.css has no shared `a{color:var(--accent-ink)}` link rule")
    for sel, body in rules:
        if not re.search(r"(^|;)color:", body.strip()):
            continue
        for one in (x.strip() for x in sel.split(",")):
            is_link = re.search(r"(^| |>)a(\[[^]]*\])?(:[a-z-]+)?$", one)
            if is_link and one != "a" and not one.startswith(CHROME):
                bad.append(f"site.css: `{one}` sets its own link color")
    pushed = any(
        sel.startswith("footer") and ".edit" in sel and "margin-left:auto" in body
        for sel, body in rules
    )
    if not pushed:
        bad.append("site.css does not push the footer's Edit link to the right")
    return bad


def footer_problems(where: str, html: str) -> list[str]:
    m = re.search(r"<footer>(.*?)</footer>", html, flags=re.S)
    if not m:
        return [f"{where}: no footer"]
    foot = m.group(1)
    bad = []
    if not re.search(r"<nav aria-label=\"[^\"]+\"", foot):
        bad.append(f"{where}: the footer has no nav with an aria-label")
    links = re.findall(r'<a [^>]*href="([^"]*)"', foot)
    if len(links) != len(set(links)):
        bad.append(f"{where}: the footer links a page twice")
    if 'class="edit"' in foot and not re.search(
        r'</nav>\s*<a class="edit"[^>]*>[^<]*</a>\s*$', foot
    ):
        bad.append(f"{where}: the Edit link is not the last item, after the nav")
    return bad


def links(directory: str | None = None) -> list[str]:
    d = Path(directory) if directory else ROOT / "target" / "site"
    bad = css_problems((d / "assets" / "site.css").read_text())
    pages = sorted((d / "docs").rglob("index.html"))
    edits = 0
    for p in pages:
        html = p.read_text()
        edits += 'class="edit"' in html
        bad += footer_problems(str(p.relative_to(d)), html)
    if not edits:
        bad.append(f"{d}: no docs page carries an Edit link")
    print(f"  [{'x' if bad else '+'}] links and footer of {len(pages)} docs pages in {d}")
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
    return bad + installer(f"{repo}/releases/download/v{version}/{INSTALLER}")


def installer(url: str) -> list[str]:
    """The shell installer `/install.sh` sends people to checks a digest per archive. A dist
    change that dropped `checksum` would still ship a script, one that verifies nothing."""
    status, _, text = fetch_following(url)
    good = status == 200 and CHECKSUM in text
    print(
        f"  [{'+' if good else 'x'}] installer checks digests: {url} ({status or 'no answer'})"
    )
    if status != 200:
        return [f"shell installer does not answer at {url}"]
    if CHECKSUM not in text:
        return [
            f"shell installer at {url} has no {CHECKSUM}, so it verifies no download"
        ]
    return []


def fetch_following(url: str) -> Answer:
    """`fetch` that follows redirects: release downloads answer from a storage host."""
    for _ in range(5):
        status, headers, text = fetch(url, body=True)
        loc = headers.get("Location", "")
        if status not in (301, 302, 303, 307, 308) or not loc.startswith("https://"):
            return status, headers, text
        url = loc
    return 0, email.message.Message(), ""


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


def links_self_test() -> bool:
    """A content link with its own color, a footer without the nav and one whose Edit link
    is not after the nav are red; the shared rules and a good footer are green."""
    base = "a{color:var(--accent-ink)}footer .edit{margin-left:auto}"
    nav = '<nav aria-label="F"><a href="/a">A</a></nav>'
    edit = '<a class="edit" href="/e">Edit</a>'
    ok = (
        not css_problems(base + ".pager a{color:var(--fg)}")
        and css_problems(base + ".groups li a{color:var(--fg)}")
        and css_problems("a{color:red}")
        and not footer_problems("p", f"<footer>{nav}\n{edit}\n</footer>")
        and footer_problems("p", f"<footer>{edit}{nav}</footer>")
        and footer_problems("p", f"<footer><a href='/a'>A</a>{edit}</footer>")
    )
    if not ok:
        print("self-test: the links check misjudged a planted fault")
    return bool(ok)


def self_test() -> int:
    """An installer without a digest line, or one that does not answer, is red; one with it
    is green. `fetch` is replaced, so nothing goes to the network."""
    global fetch
    real = fetch
    ok = "x\n_checksum_value=abc\n"
    cases = [
        ((200, email.message.Message(), "install without digests\n"), True),
        ((404, email.message.Message(), ""), True),
        ((200, email.message.Message(), ok), False),
    ]
    good = True
    try:
        for answer, red in cases:
            fetch = lambda *_a, answer=answer, **_k: answer  # noqa: E731
            if bool(installer("https://example.dev/i.sh")) != red:
                print(
                    f"self-test: {answer[0]} {answer[2]!r} should be {'red' if red else 'green'}"
                )
                good = False
    finally:
        fetch = real
    good = links_self_test() and good
    print("self-test", "ok" if good else "FAILED")
    return 0 if good else 1


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    cmd = sys.argv[1] if len(sys.argv) > 1 else ""
    run = {"release": release, "domains": domains, "csp": csp, "links": links}.get(cmd)
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
