//! Every document the docs index publishes as a page of the site, one per file at
//! `/docs/<name>/`, plus the index at `/docs/` and the changelog. Every heading carries a stable
//! id (its slug), so a link can name the exact rule it means, for instance
//! `/docs/counting/#merges`. A relative link to another document becomes that page's path, and
//! one to a file the site does not publish points at the file in the repository.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

use crate::docs_index::{normalize, on_site};
use crate::out::Out;
use crate::site::changelog;
use crate::site::{THEME_INIT_JS, esc};

pub(super) const CHANGELOG: &str = "CHANGELOG.md";

/// Where pages live and what they are called, for the shell and the links.
pub(super) struct Site<'a> {
    pub home: &'a str,
    pub repo: &'a str,
    pub name: &'a str,
}

/// Where each published document is, so a relative link can be rewritten.
pub(super) struct Links<'a> {
    pages: BTreeMap<String, String>,
    repo: &'a str,
}

impl<'a> Links<'a> {
    pub(super) fn new(repo: &'a str) -> Self {
        let mut pages: BTreeMap<String, String> = on_site()
            .into_iter()
            .map(|(path, ..)| (path.to_owned(), page_path(stem(path))))
            .collect();
        pages.insert(CHANGELOG.to_owned(), "changelog/".to_owned());
        Self { pages, repo }
    }

    /// `dest`, written in the document at `from`, as a link that works on the site.
    pub(super) fn rewrite(&self, from: &str, dest: &str) -> String {
        if dest.contains("://") || dest.starts_with("mailto:") || dest.starts_with('#') {
            return dest.to_owned();
        }
        let (file, anchor) = dest.split_once('#').unwrap_or((dest, ""));
        let dir = Path::new(from).parent().unwrap_or_else(|| Path::new(""));
        let target = normalize(&dir.join(file));
        let hash = if anchor.is_empty() {
            String::new()
        } else {
            format!("#{anchor}")
        };
        match self.pages.get(&target) {
            Some(page) => format!("/{page}{hash}"),
            None if file.ends_with('/') => format!("{}/tree/main/{target}{hash}", self.repo),
            None => format!("{}/blob/main/{target}{hash}", self.repo),
        }
    }
}

fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

/// The slug of a heading: lowercase words joined by `-`, so "Code age and releases" is
/// `code-age-and-releases`.
pub fn slug(heading: &str) -> String {
    let mut out = String::new();
    for c in heading.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

/// The site path of a page, from its file stem.
pub fn page_path(stem: &str) -> String {
    format!("docs/{stem}/")
}

/// The heading slugs of one markdown text, in order.
#[cfg(test)]
pub fn anchors(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for ev in Parser::new_ext(text, Options::empty()) {
        match ev {
            Event::Start(Tag::Heading { .. }) => cur = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => out.extend(cur.take().map(|h| slug(&h))),
            Event::Text(t) | Event::Code(t) => {
                if let Some(h) = cur.as_mut() {
                    h.push_str(&t);
                }
            }
            _ => {}
        }
    }
    out
}

/// The markdown of the document at `from` as HTML: each heading with the id `id` gives it (given
/// its level and text) and each relative link pointed at its page.
pub(super) fn body(
    text: &str,
    from: &str,
    links: &Links,
    id: &mut dyn FnMut(usize, &str) -> String,
) -> String {
    let mut events: Vec<Event> = Vec::new();
    let mut pending: Vec<Event> = Vec::new();
    let mut heading: Option<(HeadingLevel, String)> = None;
    for ev in Parser::new_ext(text, Options::empty()) {
        let ev = match ev {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Link {
                link_type,
                dest_url: CowStr::from(links.rewrite(from, &dest_url)),
                title,
                id,
            }),
            ev => ev,
        };
        match (&mut heading, ev) {
            (None, Event::Start(Tag::Heading { level, .. })) => {
                heading = Some((level, String::new()));
            }
            (Some((level, text)), Event::End(TagEnd::Heading(_))) => {
                events.push(Event::Start(Tag::Heading {
                    level: *level,
                    id: Some(CowStr::from(id(*level as usize, text))),
                    classes: vec![],
                    attrs: vec![],
                }));
                events.append(&mut pending);
                events.push(Event::End(TagEnd::Heading(*level)));
                heading = None;
            }
            (Some((_, text)), ev) => {
                if let Event::Text(t) | Event::Code(t) = &ev {
                    text.push_str(t);
                }
                pending.push(ev);
            }
            (None, ev) => events.push(ev),
        }
    }
    let mut out = String::new();
    html::push_html(&mut out, events.into_iter());
    out
}

/// A whole page: the head, a way back, `inner` and a footer with `foot` (HTML, may be empty).
pub(super) fn shell(site: &Site, title: &str, path: &str, inner: &str, foot: &str) -> String {
    let footer = if foot.is_empty() {
        String::new()
    } else {
        format!("<footer>\n{foot}\n</footer>\n")
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{t} - {s}</title>\n<link rel=\"canonical\" href=\"{home}/{path}\">\n\
         <link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/mark.svg\">\n\
         <link rel=\"stylesheet\" href=\"/assets/site.css\">\n\
         <script>{THEME_INIT_JS}</script>\n</head>\n<body>\n<main>\n\
         <p><a href=\"/\">{s}</a> / <a href=\"/docs/\">Docs</a></p>\n{inner}{footer}\
         </main>\n</body>\n</html>\n",
        t = esc(title),
        s = esc(site.name),
        home = esc(site.home),
    )
}

/// The link to a file in the repository, as a footer entry.
pub(super) fn source_link(site: &Site, rel: &str) -> String {
    format!(
        "<a href=\"{}/blob/main/{rel}\">Source of this page</a>",
        esc(site.repo)
    )
}

/// The docs index: every published document under the heading of its kind.
fn index(docs: &[(String, String)]) -> String {
    let mut out = String::from("<h1>Documentation</h1>\n");
    let mut group = "";
    for (path, title, heading, what) in on_site()
        .into_iter()
        .map(|(p, h, w)| (p, docs.iter().find(|d| d.0 == p), h, w))
        .filter_map(|(p, d, h, w)| d.map(|d| (p, d.1.as_str(), h, w)))
    {
        if group != heading {
            if !group.is_empty() {
                out.push_str("</ul>\n");
            }
            let _ = writeln!(
                out,
                "<h2 id=\"{}\">{}</h2>\n<ul>",
                slug(heading),
                esc(heading)
            );
            group = heading;
        }
        let _ = writeln!(
            out,
            "<li><a href=\"/{}\">{}</a>: {}</li>",
            page_path(stem(path)),
            esc(title),
            esc(&what.replace('`', "")),
        );
    }
    out.push_str("</ul>\n");
    out
}

/// One page per published document, the docs index and the changelog. Returns the files and
/// every page path, for the sitemap and the link check.
pub fn pages(root: &Path, home: &str, repo: &str, name: &str) -> Result<(Vec<Out>, Vec<String>)> {
    let site = Site { home, repo, name };
    let links = Links::new(repo);
    let read = |rel: &str| {
        std::fs::read_to_string(root.join(rel)).with_context(|| format!("reading {rel}"))
    };
    let mut outs = Vec::new();
    let mut paths = vec!["docs/".to_owned()];
    let mut titles = Vec::new();
    for (rel, ..) in on_site() {
        let text = read(rel)?;
        let title = text
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .with_context(|| format!("{rel} has no title"))?
            .to_owned();
        let path = page_path(stem(rel));
        if paths.contains(&path) {
            anyhow::bail!("{rel}: another document is already the page {path}");
        }
        let inner = body(&text, rel, &links, &mut |_, h| slug(h));
        let page = shell(&site, &title, &path, &inner, &source_link(&site, rel));
        outs.push(Out::file(format!("site/{path}index.html"), page));
        paths.push(path);
        titles.push((rel.to_owned(), title));
    }
    let inner = index(&titles);
    outs.push(Out::file(
        "site/docs/index.html",
        shell(&site, "Documentation", "docs/", &inner, ""),
    ));
    outs.push(changelog::page(&site, &links, &read(CHANGELOG)?));
    paths.push(changelog::PATH.to_owned());
    paths.sort();
    Ok((outs, paths))
}
