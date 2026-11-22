#!/usr/bin/env python3
"""CONV-DOCS-A11Y: markdown that reads well with a screen reader and a keyboard.

Four checks over every markdown file in the product's `[a11y] docs` globs:
  1. Heading levels never skip (an h2 may follow an h1, not an h3), and a page has exactly one h1.
  2. Every image has alt text that says what it shows: not empty, not a file name, not a bare
     placeholder word such as "image" or "screenshot".
  3. Link text is never "here", "click here", "link" or a bare URL.
  4. Every fenced code block names its language.
Text inside a fence is never read for the first three. A file with no heading at all has no h1 to check. `[a11y] h1_required = false` allows a page
with headings and no h1.
`[a11y] known` lists accepted problems as "path: problem" (the hit without its line number). It
can only shrink: an entry that no longer matches a hit fails the lint until it is removed.

  docs-a11y.py [--repo DIR] [<file>...]   the tree, or only these files
The self-test runs on every invocation: a check that stops firing is a dead guard.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True  # the copy in a product stays equal to its manifest
sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
import config  # noqa: E402

RULE = "CONV-DOCS-A11Y"
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})\s*(.*)$")
HTML_H = re.compile(r"^\s*<h([1-6])\b", re.I)
HEADING = re.compile(r"^ {0,3}(#{1,6})(?:\s+(.*?))?\s*#*\s*$")
IMAGE = re.compile(r"!\[([^\]]*)\]\(([^)\s]*)[^)]*\)")
HTML_IMG = re.compile(r"<img\b[^>]*>", re.I)
LINK = re.compile(r"(?<!!)\[([^\]]*)\]\([^)]*\)")
AUTOLINK = re.compile(r"<(https?://[^>\s]+)>")
CODE_SPAN = re.compile(r"`[^`]*`")
FILE_ALT = re.compile(r"\.(png|jpe?g|gif|svg|webp|avif|bmp|tiff?)$", re.I)
PLACEHOLDER_ALT = {"image", "img", "picture", "photo", "graphic", "screenshot", "icon", "figure", "untitled"}
BAD_LINK = {"here", "click here", "link", "this link", "this", "more", "read more"}
URL = re.compile(r"^(https?://|www\.)\S+$", re.I)


def alt_problem(alt: str, src: str) -> str | None:
    text = alt.strip()
    if not text:
        return "an image with no alt text"
    base = src.rsplit("/", 1)[-1]
    if FILE_ALT.search(text) or (base and text.lower() in (base.lower(), base.rsplit(".", 1)[0].lower())):
        return f"alt text {text!r} is a file name"
    if text.lower() in PLACEHOLDER_ALT:
        return f"alt text {text!r} does not say what the image shows"
    return None


def link_problem(text: str) -> str | None:
    t = re.sub(r"\s+", " ", text.strip()).lower()
    if t in BAD_LINK:
        return f"link text {text.strip()!r} means nothing out of context"
    if URL.match(text.strip()):
        return f"link text {text.strip()[:60]!r} is a bare URL"
    return None


def scan(path: str, body: str, h1_required: bool = True) -> list[str]:
    bad: list[str] = []
    fence: tuple[str, int] | None = None
    prev = 0
    h1s: list[int] = []
    lines = body.splitlines()
    start = 0
    # A frontmatter `title:` is the page's h1 (Starlight and similar renderers draw it as one).
    if lines and lines[0].strip() == "---":
        end = next((i for i in range(1, len(lines)) if lines[i].strip() == "---"), None)
        if end is not None:
            start = end + 1
            if any(re.match(r"title\s*:\s*\S", x) for x in lines[1:end]):
                h1s.append(1)
                prev = 1
    for n, line in enumerate(lines, 1):
        if n <= start:
            continue
        m = FENCE.match(line)
        if fence:
            if m and m.group(1)[0] == fence[0] and len(m.group(1)) >= fence[1] and not m.group(2):
                fence = None
            continue
        if m:
            fence = (m.group(1)[0], len(m.group(1)))
            if not m.group(2).strip():
                bad.append(f"{path}:{n}: a fenced code block with no language")
            continue
        h = HEADING.match(line)
        hh = HTML_H.match(line)
        if h or hh:
            level = len(h.group(1)) if h else int(hh.group(1))
            if level == 1:
                h1s.append(n)
            if level > prev + 1:
                bad.append(f"{path}:{n}: h{level} follows {'h' + str(prev) if prev else 'no heading'}, a level is skipped")
            prev = level
            continue
        text = CODE_SPAN.sub("", line)
        for img in IMAGE.finditer(text):
            if (why := alt_problem(img.group(1), img.group(2))):
                bad.append(f"{path}:{n}: {why}")
        for tag in HTML_IMG.finditer(text):
            alt = re.search(r"""\balt\s*=\s*(?:"([^"]*)"|'([^']*)')""", tag.group(0), re.I)
            src = re.search(r"""\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)')""", tag.group(0), re.I)
            alt_text = (alt.group(1) or alt.group(2) or "") if alt else ""
            src_text = (src.group(1) or src.group(2) or "") if src else ""
            if (why := alt_problem(alt_text, src_text)):
                bad.append(f"{path}:{n}: {why}")
        for link in LINK.finditer(text):
            if (why := link_problem(link.group(1))):
                bad.append(f"{path}:{n}: {why}")
        for auto in AUTOLINK.finditer(text):
            bad.append(f"{path}:{n}: link text {auto.group(1)[:60]!r} is a bare URL")
    if h1_required and (h1s or prev) and not h1s:
        bad.append(f"{path}:1: the page has no h1")
    if len(h1s) > 1:
        bad.append(f"{path}:{h1s[1]}: a second h1, a page has one")
    return bad


def self_test() -> bool:
    ok = True
    page = "# T\n\n## A\n\ntext\n"
    must_hit = {
        "a skipped level": "# T\n\n### A\n",
        "a second h1": "# T\n\n# U\n",
        "no h1": "## A\n",
        "empty alt": f"{page}\n![](a.svg)\n",
        "file name alt": f"{page}\n![shot.png](a.png)\n",
        "placeholder alt": f"{page}\n![screenshot](a.png)\n",
        "html empty alt": f'{page}\n<img src="a.png" alt="">\n',
        "link here": f"{page}\nSee [here](x.md).\n",
        "link this": f"{page}\nSee [this link](x.md).\n",
        "link click here": f"{page}\n[Click here](x.md)\n",
        "link bare url": f"{page}\n[https://example.invalid/a](https://example.invalid/a)\n",
        "autolink": f"{page}\n<https://example.invalid/a>\n",
        "bare fence": f"{page}\n```\nx\n```\n",
        "bare tilde fence": f"{page}\n~~~\nx\n~~~\n",
        "an h1 under a frontmatter title": "---\ntitle: T\n---\n\n# U\n",
        "an h3 under a frontmatter title": "---\ntitle: T\n---\n\n### A\n",
    }
    must_pass = {
        "a clean page": f"{page}\n![A bar chart of commits per week](c.svg)\n\n[the install guide](x.md)\n\n```sh\nls\n```\n",
        "code is not read": f"{page}\n```md\n### skipped\n![](a.png)\n[here](x)\n```\n",
        "a closing fence": f"{page}\n```sh\nls\n```\n\ntext\n",
        "code span": f"{page}\nThe syntax is `[here](x)`.\n",
        "html alt": f'{page}\n<img src="a.png" alt="Graph of load over a day">\n',
        "no heading at all": "just text\n",
        "an html h1": '<h1 align="center"><picture><img src="l.svg" alt="The product logo"></picture></h1>\n\n## A\n',
        "a frontmatter title is the h1": "---\ntitle: T\ndescription: d\n---\n\ntext\n\n## A\n",
    }
    for why, body in must_hit.items():
        if not scan("docs/a.md", body):
            print(f"FAIL [{RULE}] self-test: {why} must be refused", file=sys.stderr)
            ok = False
    for why, body in must_pass.items():
        if (hits := scan("docs/a.md", body)):
            print(f"FAIL [{RULE}] self-test: {why} must pass, got {hits}", file=sys.stderr)
            ok = False
    return ok


def main(argv: list[str]) -> int:
    root, args = config.take_repo(argv)
    cfg = config.load(root)
    if not self_test():
        return 2
    globs = cfg["a11y"]["docs"]
    files = args or [f for f in config.tracked(root, cfg["scope"]["exclude"]) if config.matches(f, globs)]
    bad: list[str] = []
    seen = 0
    for f in files:
        p = root / f
        if not f.endswith(".md") or not config.matches(f, globs) or not p.is_file():
            continue
        seen += 1
        bad += scan(f, p.read_text(errors="replace"), cfg["a11y"]["h1_required"])
    known = set(cfg["a11y"]["known"])
    keyed = [(re.sub(r"^([^:]+):\d+: ", r"\1: ", b), b) for b in bad]
    used = {k for k, _ in keyed if k in known}
    bad = [b for k, b in keyed if k not in known]
    stale = [] if args else sorted(known - used)
    bad += [f"{k}: listed in [a11y] known but fixed, remove the entry" for k in stale]
    if not args and seen < cfg["scope"]["min_files"]:
        print(f"FAIL [{RULE}] scanned only {seen} file(s): the census is broken")
        return 2
    note = f", {len(used)} known problem(s) left" if used else ""
    return config.report(RULE, bad, f"{len(bad)} accessibility problem(s) in the docs",
                         f"headings, alt text, link text and code fences in {seen} file(s){note}",
                         "One h1, no skipped level, alt text that says what the image shows, link text that",
                         "makes sense alone, and a language on every fenced block.")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
