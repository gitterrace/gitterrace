"""The agent-facing files of the site, checked by `site-check.py agents`: robots.txt, llms.txt,
the markdown twins, the JSON-LD and the sitemap dates. Stdlib only, imported by site-check.py."""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
CANONICAL = "gitterrace.dev"


# The bots that fetch a page to answer a person (search indexes and user-triggered fetches). A
# robots.txt that shuts one out makes the site vanish from that answer without any error.
OPEN_BOTS = ("OAI-SearchBot", "Claude-SearchBot", "Claude-User", "Googlebot")


def robots_problems(text: str) -> list[str]:
    """Each of OPEN_BOTS must be allowed the whole site by the group that applies to it (its own,
    else `*`), and the file must name the sitemap."""
    groups: list[tuple[list[str], list[str]]] = []
    agents: list[str] = []
    rules: list[str] = []
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        key, _, val = line.partition(":")
        key, val = key.strip().lower(), val.strip()
        if key == "user-agent":
            if rules:
                groups.append((agents, rules))
                agents, rules = [], []
            agents.append(val.lower())
        elif key in ("allow", "disallow") and agents:
            rules.append(f"{key}:{val}")
    if agents:
        groups.append((agents, rules))
    bad = []
    for bot in OPEN_BOTS:
        mine = [r for a, r in groups if bot.lower() in a] or [r for a, r in groups if "*" in a]
        if any(x in ("disallow:/", "disallow:/*") for r in mine for x in r):
            bad.append(f"robots.txt disallows {bot}")
    sitemap = f"https://{CANONICAL}/sitemap.xml"
    if not re.search(r"^sitemap:\s*" + re.escape(sitemap) + r"\s*$", text, re.I | re.M):
        bad.append(f"robots.txt has no `Sitemap: {sitemap}` line")
    return bad


def workspace_version() -> str:
    """The version of Cargo.toml's [workspace.package] (no tomllib before Python 3.11)."""
    text = (ROOT / "Cargo.toml").read_text()
    block = text.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
    m = re.search(r'^version\s*=\s*"([^"]+)"', block, re.M)
    return m.group(1) if m else ""


def source_of(path: str, html: str) -> str:
    """The repository file a sitemap page (`docs/cli/`, `""` for home) is rendered from."""
    if path == "":
        return "docs/landing.md"
    if path == "docs/":
        return "docs/README.md"
    if path == "changelog/":
        return "CHANGELOG.md"
    m = re.search(r'class="edit"\s+href="[^"]*/(?:blob|edit)/main/([^"]+)"', html)
    if m:
        return m.group(1)
    if path.startswith("legal/"):
        return f"docs/{path.rstrip('/')}.md"
    return f"docs/{path.rstrip('/').removeprefix('docs/')}.md"


def sitemap_entries(xml: str) -> list[tuple[str, str]]:
    """(site path, the entry's markup after its loc) of every <url>."""
    out = []
    for loc, rest in re.findall(r"<url><loc>https://[^/<]+/([^<]*)</loc>(.*?)</url>", xml, re.S):
        out.append((loc, rest))
    return out


def llms_problems(llms: str, paths: list[str], version: str) -> list[str]:
    """llms.txt names the workspace version and links every sitemap page: a document by its
    index.md, the home page and the docs index by the page."""
    bad = []
    if f"Version: {version}" not in llms.splitlines():
        bad.append(f"llms.txt does not say `Version: {version}`, the workspace version")
    for p in paths:
        want = p if p in ("", "docs/") else p + "index.md"
        if not re.search(r"\]\(https://[^/)]+/" + re.escape(want) + r"\)", llms):
            bad.append(f"llms.txt does not link /{want}, a page of the sitemap")
    return bad


def json_ld_problems(html: str, version: str) -> list[str]:
    import json

    blocks = re.findall(r'<script type="application/ld\+json">(.*?)</script>', html, re.S)
    if len(blocks) != 1:
        return [f"the landing page has {len(blocks)} JSON-LD blocks, want exactly one"]
    try:
        doc = json.loads(blocks[0])
    except ValueError as e:
        return [f"the landing page JSON-LD is not JSON: {e}"]
    bad = []
    if doc.get("@type") != "SoftwareApplication":
        bad.append("the JSON-LD is not a SoftwareApplication")
    if doc.get("softwareVersion") != version:
        bad.append(f"JSON-LD softwareVersion {doc.get('softwareVersion')!r} is not {version}")
    for key in ("aggregateRating", "review", "mainEntity", "step"):
        if key in doc:
            bad.append(f"the JSON-LD carries {key}: no ratings, FAQ or how-to markup")
    return bad


def sitemap_problems(entries: list[tuple[str, str]], lastmod: dict[str, str]) -> list[str]:
    """`lastmod` is the date of the last commit of each entry's source, by site path."""
    bad = []
    for path, rest in entries:
        if re.search(r"<(changefreq|priority)>", rest):
            bad.append(f"sitemap /{path}: changefreq or priority, which search engines ignore")
        m = re.search(r"<lastmod>([^<]*)</lastmod>", rest)
        if not m:
            bad.append(f"sitemap /{path}: no lastmod")
        elif path in lastmod and m.group(1) != lastmod[path]:
            bad.append(f"sitemap /{path}: lastmod {m.group(1)} is not its source's {lastmod[path]}")
    return bad


def commit_date(rel: str) -> str:
    import datetime

    out = subprocess.run(
        ["git", "log", "-1", "--format=%ct", "--", rel],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()
    if not out:
        return ""
    day = datetime.datetime.fromtimestamp(int(out), datetime.timezone.utc)
    return day.strftime("%Y-%m-%d")


def agents(directory: str | None = None) -> list[str]:
    d = Path(directory) if directory else ROOT / "target" / "site"
    for f in ("robots.txt", "sitemap.xml", "llms.txt", "llms-full.txt", "_headers", "index.html"):
        if not (d / f).is_file():
            return [f"{d / f} is not built: run `cargo xtask site`"]
    bad = robots_problems((d / "robots.txt").read_text())
    xml = (d / "sitemap.xml").read_text()
    entries = sitemap_entries(xml)
    paths = [p for p, _ in entries]
    version = workspace_version()
    llms = (d / "llms.txt").read_text()
    bad += llms_problems(llms, paths, version)
    for link in re.findall(r"\]\(https://[^/)]+/([^)]*index\.md)\)", llms):
        if not (d / link).is_file():
            bad.append(f"llms.txt links /{link}, which the site does not build")
    full = (d / "llms-full.txt").read_text()
    if f"Version: {version}" not in full.splitlines():
        bad.append(f"llms-full.txt does not say `Version: {version}`")
    headers = (d / "_headers").read_text()
    lastmod = {}
    for path, _ in entries:
        page = d / path / "index.html" if path else d / "index.html"
        html = page.read_text()
        src = source_of(path, html)
        if path:
            md = d / path / "index.md"
            if path != "docs/" and md.is_file() and md.read_text() != (ROOT / src).read_text():
                bad.append(f"/{path}index.md is not {src} as it is in the repository")
            if path != "docs/":
                if not md.is_file():
                    bad.append(f"/{path}index.md is not built")
                alt = f'<link rel="alternate" type="text/markdown" href="/{path}index.md">'
                if alt not in html:
                    bad.append(f"/{path}: no {alt}")
                link = f'Link: </{path}index.md>; rel="alternate"; type="text/markdown"'
                if link not in headers:
                    bad.append(f"_headers: no {link} for /{path}")
        lastmod[path] = commit_date(src)
    bad += json_ld_problems((d / "index.html").read_text(), version)
    bad += sitemap_problems(entries, lastmod)
    print(f"  [{'x' if bad else '+'}] robots, llms, markdown twins, JSON-LD and sitemap of {d}")
    return bad


def agent_files_self_test() -> bool:
    """A page missing from llms.txt, a stale version, a drifted or doubled JSON-LD, a rating,
    a missing or wrong lastmod and a changefreq are red; the matching files are green."""
    ld = (
        '<script type="application/ld+json">'
        '{"@type":"SoftwareApplication","softwareVersion":"1.0"}</script>'
    )
    llms = "# n\n\nVersion: 1.0\n\n- [a](https://h.dev/)\n- [b](https://h.dev/docs/x/index.md)\n"
    entries = [("", "<lastmod>2026-01-02</lastmod>")]
    return (
        bool(
            not llms_problems(llms, ["", "docs/x/"], "1.0")
            and llms_problems(llms, ["", "docs/y/"], "1.0")
            and llms_problems(llms, [""], "1.1")
            and not json_ld_problems(ld, "1.0")
            and json_ld_problems(ld, "1.1")
            and json_ld_problems(ld + ld, "1.0")
            and json_ld_problems(ld.replace("}</", ',"aggregateRating":{}}</'), "1.0")
            and not sitemap_problems(entries, {"": "2026-01-02"})
            and sitemap_problems(entries, {"": "2026-01-03"})
            and sitemap_problems([("", "")], {})
            and sitemap_problems([("", entries[0][1] + "<changefreq>daily</changefreq>")], {})
        )
        or print("self-test: the agent files check misjudged a planted fault")
        or False
    )


def agents_self_test() -> bool:
    """A planted Disallow for a named bot or for `*`, and a missing Sitemap line, are red."""
    ok_txt = f"User-agent: *\nAllow: /\nSitemap: https://{CANONICAL}/sitemap.xml\n"
    return (
        bool(
            not robots_problems(ok_txt)
            and robots_problems(ok_txt.replace("Allow: /", "Disallow: /"))
            and robots_problems(ok_txt + "User-agent: Claude-User\nDisallow: /\n")
            and not robots_problems(ok_txt + "User-agent: GPTBot\nDisallow: /\n")
            and robots_problems(ok_txt.split("Sitemap")[0])
        )
        or print("self-test: the robots check misjudged a planted fault")
        or False
    )
