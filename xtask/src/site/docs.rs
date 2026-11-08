//! Every document the docs index publishes as a page of the site, one per file at
//! `/docs/<name>/`, plus the index at `/docs/` and the changelog. Every heading carries a stable
//! id (its slug), so a link can name the exact rule it means, for instance
//! `/docs/architecture/#cache`. A relative link to another document becomes that page's path, and
//! one to a file the site does not publish points at the file in the repository.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use pulldown_cmark::{CowStr, Event, HeadingLevel, Parser, Tag, TagEnd, html};

use crate::docs_index::{LEGAL, normalize, on_site};
use crate::out::Out;
use crate::site::changelog;
use crate::site::{THEME_INIT_JS, THEME_TOGGLE_JS, body_open, esc, footer, outline_script};

pub(super) const CHANGELOG: &str = "CHANGELOG.md";

/// Where pages live and what they are called, for the shell and the links.
pub(super) struct Site<'a> {
    pub home: &'a str,
    pub repo: &'a str,
    pub name: &'a str,
    pub demo: &'a str,
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
        for (rel, _) in LEGAL {
            pages.insert(rel.to_owned(), legal_path(rel));
        }
        Self { pages, repo }
    }

    /// `dest`, written in the document at `from`, as a link that works on the site. An image the
    /// site does not carry is loaded from the repository, as raw bytes.
    pub(super) fn rewrite(&self, from: &str, dest: &str, image: bool) -> String {
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
            None if image => format!("{}/raw/main/{target}", self.repo),
            None if file.ends_with('/') => format!("{}/tree/main/{target}{hash}", self.repo),
            None => format!("{}/blob/main/{target}{hash}", self.repo),
        }
    }
}

impl Links<'_> {
    /// Raw HTML (a `<picture>`) with each `src` and `srcset` pointed at where the file is.
    fn rewrite_html(&self, from: &str, html: &str) -> String {
        let mut out = html.to_owned();
        for attr in ["src", "srcset"] {
            let pat = format!(" {attr}=\"");
            let mut done = String::new();
            let mut rest = out.as_str();
            while let Some((before, after)) = rest.split_once(&pat) {
                let (value, tail) = after.split_once('"').unwrap_or((after, ""));
                done.push_str(before);
                let _ = write!(done, "{pat}{}\"", self.rewrite(from, value, true));
                rest = tail;
            }
            done.push_str(rest);
            out = done;
        }
        out
    }
}

pub(super) fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

/// The slug of a heading, by the rule the app and the repository host use: lowercase, keep
/// letters, digits, `-` and `_`, spaces to `-`. So "Code age and releases" is
/// `code-age-and-releases` and "HTTP (/api/v1)" is `http-apiv1`, wherever the page is read.
pub fn slug(heading: &str) -> String {
    heading
        .trim()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c == '-' || c == '_' || c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Fails when two elements of one page share an id, so a link cannot mean two places.
pub fn check_unique_ids(path: &str, html: &str) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for id in html
        .split(" id=\"")
        .skip(1)
        .filter_map(|r| r.split_once('"').map(|v| v.0))
    {
        if !seen.insert(id) {
            anyhow::bail!("{path}: more than one heading has the id {id}");
        }
    }
    Ok(())
}

/// The site path of a legal page: `legal/<stem>/`, the same on every site.
pub fn legal_path(rel: &str) -> String {
    format!("legal/{}/", stem(rel))
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
    for ev in Parser::new_ext(text, gitterrace_web::markdown_options()) {
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
    for ev in Parser::new_ext(text, gitterrace_web::markdown_options()) {
        let ev = match ev {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Link {
                link_type,
                dest_url: CowStr::from(links.rewrite(from, &dest_url, false)),
                title,
                id,
            }),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => Event::Start(Tag::Image {
                link_type,
                dest_url: CowStr::from(links.rewrite(from, &dest_url, true)),
                title,
                id,
            }),
            Event::Html(h) => Event::Html(CowStr::from(links.rewrite_html(from, &h))),
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
    wrap_wide(&out)
}

/// The site's lines at most `WIDTH` wide where a space allows: a line is broken at the last
/// space before the width, outside a tag and outside `<pre>`, where a newline reads as the space
/// it replaces. The pages are source files too, held to the repository's line width.
fn wrap_wide(html: &str) -> String {
    const WIDTH: usize = 100;
    let mut out = String::with_capacity(html.len());
    let mut pre = false;
    for line in html.split_inclusive('\n') {
        if line.contains("<pre") {
            pre = true;
        }
        if pre || line.trim_end().chars().count() <= WIDTH {
            out.push_str(line);
        } else {
            out.push_str(&break_line(line, WIDTH));
        }
        if line.contains("</pre>") {
            pre = false;
        }
    }
    out
}

/// The tags a table is built of: a newline before one is whitespace a table drops, so a table
/// row with no spaces can still break between its cells.
const TABLE_TAGS: [&str; 10] = [
    "<thead", "</thead", "<tbody", "</tbody", "<tr", "</tr", "<th", "</th", "<td", "</td",
];

/// One line broken at spaces outside tags, or before a table tag, each piece at most `width`
/// wide where it can be.
fn break_line(line: &str, width: usize) -> String {
    let mut out = String::new();
    // `last` is where the line may break and how many bytes the break replaces: a space is
    // replaced by the newline, a table tag keeps its place after it.
    let (mut start, mut last, mut tag, mut col) = (0, None, false, 0);
    for (i, ch) in line.char_indices() {
        match ch {
            '<' => {
                let rest = &line[i..];
                let at_tag = TABLE_TAGS.iter().any(|t| {
                    rest.starts_with(t) && matches!(rest.as_bytes().get(t.len()), Some(b'>' | b' '))
                });
                if at_tag && i > start {
                    last = Some((i, 0));
                }
                tag = true;
            }
            '>' => tag = false,
            ' ' if !tag => last = Some((i, 1)),
            _ => {}
        }
        col += 1;
        if col > width
            && let Some((at, skip)) = last.take()
        {
            out.push_str(&line[start..at]);
            out.push('\n');
            start = at + skip;
            col = line[start..=i].chars().count();
        }
    }
    out.push_str(&line[start..]);
    out
}

/// A whole page: the head, the bar, `inner`, a footer with `foot` (HTML, may be empty) and the
/// outline `rail` (HTML, empty for none), the app's own, beside the text on a wide screen.
pub(super) fn shell(
    site: &Site,
    title: &str,
    path: &str,
    inner: &str,
    foot: &str,
    rail: &str,
) -> String {
    let footer = footer(site.repo, foot);
    let (close, script) = if rail.is_empty() {
        (String::new(), String::new())
    } else {
        (
            format!("{rail}\n</div>\n"),
            format!("<script>{}</script>\n", outline_script()),
        )
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
         <meta name=\"color-scheme\" content=\"light dark\">\n\
         <title>{t} - {s}</title>\n<link rel=\"canonical\" href=\"{home}/{path}\">\n\
         <link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/mark.svg\">\n\
         <link rel=\"stylesheet\" href=\"/assets/site.css\">\n\
         <script>{THEME_INIT_JS}</script>\n</head>\n{open}\
         {inner}{footer}\
         </main>\n{close}<script>{THEME_TOGGLE_JS}</script>\n{script}</body>\n</html>\n",
        t = esc(title),
        s = esc(site.name),
        home = esc(site.home),
        open = body_open(site.name, site.repo, site.demo, !rail.is_empty()),
    )
}

/// The "On this page" rail of a document from its level 2 headings as (id, text): the app's
/// outline, so the site and the app list headings the same way. Empty below three headings.
pub(super) fn outline(h2s: &[(String, String)]) -> String {
    let items: Vec<(&str, &str)> = h2s.iter().map(|(i, t)| (i.as_str(), t.as_str())).collect();
    // The app writes the rail on one line; the site's pages are held to the line width and
    // length. Breaking between tags adds no whitespace, so the tags are packed into lines as
    // full as the width allows.
    let mut out = String::new();
    let mut line = 0;
    for piece in gitterrace_web::outline_rail(&items).split_inclusive(['>']) {
        let n = piece.chars().count();
        if line > 0 && line + n > 100 {
            out.push('\n');
            line = 0;
        }
        out.push_str(piece);
        line += n;
    }
    // The filter field's attributes go on two lines; whitespace between attributes is free.
    out.trim_end()
        .replace("\" aria-label=\"Filter", "\"\naria-label=\"Filter")
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
pub fn pages(
    root: &Path,
    home: &str,
    repo: &str,
    name: &str,
    demo: &str,
) -> Result<(Vec<Out>, Vec<String>)> {
    let site = Site {
        home,
        repo,
        name,
        demo,
    };
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
        let mut h2s = Vec::new();
        let inner = body(&text, rel, &links, &mut |level, h| {
            let id = slug(h);
            if level == 2 {
                h2s.push((id.clone(), h.to_owned()));
            }
            id
        });
        let rail = outline(&h2s);
        let page = shell(
            &site,
            &title,
            &path,
            &inner,
            &source_link(&site, rel),
            &rail,
        );
        check_unique_ids(&path, &page)?;
        outs.push(Out::file(format!("site/{path}index.html"), page));
        paths.push(path);
        titles.push((rel.to_owned(), title));
    }
    for (rel, _) in LEGAL {
        let text = read(rel)?;
        let title = text
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .with_context(|| format!("{rel} has no title"))?
            .to_owned();
        let path = legal_path(rel);
        let inner = body(&text, rel, &links, &mut |_, h| slug(h));
        let page = shell(&site, &title, &path, &inner, &source_link(&site, rel), "");
        check_unique_ids(&path, &page)?;
        outs.push(Out::file(format!("site/{path}index.html"), page));
        paths.push(path);
    }
    let inner = index(&titles);
    outs.push(Out::file(
        "site/docs/index.html",
        shell(&site, "Documentation", "docs/", &inner, "", ""),
    ));
    let log = changelog::page(&site, &links, &read(CHANGELOG)?);
    if let Out::File { body, .. } = &log {
        check_unique_ids(changelog::PATH, body)?;
    }
    outs.push(log);
    paths.push(changelog::PATH.to_owned());
    paths.sort();
    Ok((outs, paths))
}
