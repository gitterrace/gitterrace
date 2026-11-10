//! Guards over the built site, run by `cargo xtask check` on `site/` as it is on disk. Each
//! catches a class of defect, not one instance, and has a planted red case in the tests:
//!
//! 1. a link to the markdown source of a page the site publishes, instead of the page;
//! 3. a paragraph or list item of prose that two pages both carry, outside the text a page
//!    quotes on purpose (marked `data-from`);
//! 4. a page with too little text, or no section, to be worth its link;
//! 6. a fenced block in a language that is meant to be drawn, shown as source;
//! 7. a page without a title, a meta description or the shared bar, main and footer;
//! 8. a docs page missing from the docs index, the docs list or the previous and next chain.
//!
//! Guard 2 (a dead internal link or anchor) is `disk::dead_links`, and guard 5 (sideways scroll
//! at 390 px) is the `site-bar` browser check, which measures it in a real browser.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::site::attrs;

/// The fewest words a page's main text needs, outside its navigation.
const MIN_WORDS: usize = 60;
/// The shortest prose that counts as repeated: shorter lines (a label, a link) may repeat.
const MIN_REPEAT: usize = 60;
/// Fenced languages that stand for a picture, which the site draws or must not show raw.
const DRAWN: [&str; 6] = ["mermaid", "dot", "graphviz", "plantuml", "math", "latex"];

/// What the guards need to know about the site besides its pages.
pub(super) struct Facts<'a> {
    /// The repository's web URL, the prefix of a `/blob/main/` link.
    pub repo: &'a str,
    /// Every repository file the site publishes as a page, by its path.
    pub published: BTreeSet<String>,
    /// The docs pages in index order, as site paths (`docs/cli/`).
    pub docs: Vec<String>,
}

/// Every problem the guards find on `pages`, each as one line.
pub(super) fn problems(pages: &BTreeMap<String, String>, facts: &Facts) -> Vec<String> {
    // A page is fitted to the line width, so a tag may break before an attribute. The guards
    // read each tag as if on one line.
    let joined: BTreeMap<String, String> = pages
        .iter()
        .map(|(p, h)| (p.clone(), h.replace("\n ", " ")))
        .collect();
    let pages = &joined;
    let mut bad = Vec::new();
    bad.extend(markdown_links(pages, facts));
    bad.extend(repeated_prose(pages));
    bad.extend(thin_pages(pages));
    bad.extend(raw_drawings(pages));
    bad.extend(missing_head_or_frame(pages));
    bad.extend(orphan_docs(pages, facts));
    bad
}

/// The text between the first `<open` and the matching close of `tag`, or "".
fn inside<'a>(html: &'a str, open: &str, close: &str) -> &'a str {
    html.split_once(open)
        .and_then(|(_, r)| r.split_once(close))
        .map_or("", |(m, _)| m)
}

/// `html` without its tags, entities left as they are, whitespace collapsed.
fn text(html: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => tag = true,
            '>' => {
                tag = false;
                out.push(' ');
            }
            c if !tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The page's main, without the parts that navigate (menus, previous and next) and without
/// the text it quotes from another page (`data-from`).
fn own_main(html: &str) -> String {
    // Past the `<main ...>` tag itself, so its attributes never count as text.
    let open = inside(html, "<main", "</main>");
    let mut main = open.split_once('>').map_or("", |m| m.1).to_owned();
    // The menus come first in main, right before the page's own text.
    if let Some(at) = main.find("<div class=\"menus\"") {
        let end = main[at..].find("<h1").map_or(main.len(), |e| at + e);
        main.replace_range(at..end, "");
    }
    if let Some(at) = main.find("<nav class=\"pager\"") {
        let end = main[at..]
            .find("</nav>")
            .map_or(main.len(), |e| at + e + "</nav>".len());
        main.replace_range(at..end, "");
    }
    while let Some(at) = main.find(" data-from=\"") {
        let start = main[..at].rfind('<').unwrap_or(at);
        let name: String = main[start + 1..]
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        let close = format!("</{name}>");
        let end = main[at..]
            .find(&close)
            .map_or(main.len(), |e| at + e + close.len());
        main.replace_range(start..end, "");
    }
    main
}

/// Guard 1: a link to `{repo}/blob/main/X.md` or a relative `X.md` where X is a page of the
/// site. The footer's edit link (`class="edit"`) is the one way to reach the source: `/edit/main/`
/// on a written page, `/blob/main/` labelled as the source view on a generated one.
fn markdown_links(pages: &BTreeMap<String, String>, facts: &Facts) -> Vec<String> {
    let blob = format!("{}/blob/main/", facts.repo);
    let mut bad = Vec::new();
    for (path, html) in pages {
        let html = html.replace("<a class=\"edit\" href=", "<a class=\"edit\" data-href=");
        for href in attrs(&html, "href") {
            let file = href.split('#').next().unwrap_or("");
            let rel = file.strip_prefix(&blob).unwrap_or(file);
            let local = !rel.contains("://");
            if local
                && Path::new(rel)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                && facts.published.contains(rel)
            {
                bad.push(format!(
                    "{path}: {href} links the markdown of a page the site has; link the page"
                ));
            }
        }
    }
    bad
}

/// Guard 3: prose of `MIN_REPEAT` characters or more that two pages both carry.
fn repeated_prose(pages: &BTreeMap<String, String>) -> Vec<String> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let mut bad = Vec::new();
    for (path, html) in pages {
        let main = own_main(html);
        let mut on_page = BTreeSet::new();
        for (open, close) in [("<p", "</p>"), ("<li", "</li>")] {
            for chunk in main.split(open).skip(1) {
                // `<p` and `<li` also start `<pre`, `<path` and `<link`, which are not prose.
                if !chunk.starts_with(['>', ' ']) {
                    continue;
                }
                let body = chunk.split_once('>').map_or("", |(_, b)| b);
                let line = text(body.split(close).next().unwrap_or(""));
                if line.chars().count() >= MIN_REPEAT && on_page.insert(line.clone()) {
                    match seen.get(&line) {
                        Some(first) => bad.push(format!(
                            "{path}: repeats prose of {first}: \"{}\"",
                            line.chars().take(60).collect::<String>()
                        )),
                        None => {
                            seen.insert(line, path);
                        }
                    }
                }
            }
        }
    }
    bad
}

/// Guard 4: a page whose own text is under `MIN_WORDS` words or that has no h2. The 404 is a
/// signpost and exempt.
fn thin_pages(pages: &BTreeMap<String, String>) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages.iter().filter(|p| p.0 != "404.html") {
        let main = own_main(html);
        let words = text(&main).split_whitespace().count();
        if words < MIN_WORDS || !main.contains("<h2") {
            bad.push(format!(
                "{path}: {words} words and {} h2, too thin for a page",
                main.matches("<h2").count()
            ));
        }
    }
    bad
}

/// Guard 6: a code block in a language meant to be drawn.
fn raw_drawings(pages: &BTreeMap<String, String>) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        for lang in DRAWN {
            if html.contains(&format!("class=\"language-{lang}\"")) {
                bad.push(format!(
                    "{path}: a {lang} block shows as source; draw it at build"
                ));
            }
        }
    }
    bad
}

/// Guard 7: a page without a `<title>`, a meta description of a sentence, or the shared frame.
fn missing_head_or_frame(pages: &BTreeMap<String, String>) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        let title = text(inside(html, "<title>", "</title>"));
        let desc = html
            .split_once("<meta name=\"description\" content=\"")
            .and_then(|(_, r)| r.split_once('"'))
            .map_or("", |(d, _)| d);
        if title.is_empty() {
            bad.push(format!("{path}: no title"));
        }
        if desc.split_whitespace().count() < 4 {
            bad.push(format!("{path}: no meta description of a sentence"));
        }
        for part in ["<header class=\"top\">", "<main id=\"main\"", "<footer>"] {
            if html.matches(part).count() != 1 {
                bad.push(format!("{path}: not exactly one {part}"));
            }
        }
    }
    bad
}

/// Guard 8: a docs page the docs index does not link, the docs list beside it does not list,
/// or the previous and next chain from the first page does not reach in index order.
fn orphan_docs(pages: &BTreeMap<String, String>, facts: &Facts) -> Vec<String> {
    let mut bad = Vec::new();
    let index = pages.get("docs/index.html").map_or("", String::as_str);
    let built: Vec<&str> = pages
        .keys()
        .filter_map(|p| p.strip_suffix("index.html"))
        .filter(|p| p.starts_with("docs/") && *p != "docs/")
        .collect();
    for page in &built {
        if !facts.docs.iter().any(|d| d == page) {
            bad.push(format!("{page}: a docs page the docs index does not list"));
        }
    }
    for doc in &facts.docs {
        let href = format!("href=\"/{doc}\"");
        if !index.contains(&href) {
            bad.push(format!("docs/index.html: does not link /{doc}"));
        }
        let html = pages
            .get(&format!("{doc}index.html"))
            .map_or("", String::as_str);
        let list = inside(html, "<nav class=\"docnav\"", "</nav>");
        for other in &facts.docs {
            if !list.contains(&format!("href=\"/{other}\"")) {
                bad.push(format!("{doc}: the docs list leaves out /{other}"));
            }
        }
    }
    // Walk next from the first page: it must visit every docs page, in index order.
    let mut walked = Vec::new();
    let mut at = facts.docs.first().cloned();
    while let Some(doc) = at.take() {
        if walked.contains(&doc) || walked.len() > facts.docs.len() {
            break;
        }
        let html = pages
            .get(&format!("{doc}index.html"))
            .map_or("", String::as_str);
        walked.push(doc);
        let nav = inside(html, "<nav class=\"pager\"", "</nav>");
        at = nav
            .split("<a class=\"next\" href=\"/")
            .nth(1)
            .and_then(|r| r.split_once('"'))
            .map(|(h, _)| h.to_owned());
    }
    if walked != facts.docs {
        bad.push(format!(
            "the previous and next chain is {walked:?}, not the docs index order {:?}",
            facts.docs
        ));
    }
    bad
}

#[cfg(test)]
mod tests;
