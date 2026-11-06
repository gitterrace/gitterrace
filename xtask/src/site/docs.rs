//! The user-facing internals docs (`docs/internals/*.md`) as pages of the site, one per file at
//! `/docs/<name>/`. Every heading carries a stable id (its slug), so a link can name the exact
//! rule it means, for instance `/docs/counting/#merges`.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

use crate::out::Out;
use crate::site::{OUTLINE_JS, THEME_INIT_JS, THEME_TOGGLE_JS, bar, esc, fleet};

const DIR: &str = "docs/internals";

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

/// The markdown as HTML, each heading with its slug as id.
pub fn body(text: &str) -> String {
    let mut events: Vec<Event> = Vec::new();
    let mut pending: Vec<Event> = Vec::new();
    let mut heading: Option<(HeadingLevel, String)> = None;
    for ev in Parser::new_ext(text, Options::empty()) {
        match (&mut heading, ev) {
            (None, Event::Start(Tag::Heading { level, .. })) => {
                heading = Some((level, String::new()));
            }
            (Some((level, text)), Event::End(TagEnd::Heading(_))) => {
                let id = slug(text);
                events.push(Event::Start(Tag::Heading {
                    level: *level,
                    id: Some(CowStr::from(id)),
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

/// The level 2 headings of one markdown text as (slug, title), in order.
pub fn h2s(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut cur: Option<String> = None;
    for ev in Parser::new_ext(text, Options::empty()) {
        match ev {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H2,
                ..
            }) => cur = Some(String::new()),
            Event::End(TagEnd::Heading(HeadingLevel::H2)) => {
                if let Some(h) = cur.take() {
                    out.push((slug(&h), h));
                }
            }
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

/// A page with at least this many level 2 headings gets the outline rail.
const RAIL_MIN: usize = 3;

/// The "On this page" rail, empty for a page too short to need one.
pub fn outline(items: &[(String, String)]) -> String {
    if items.len() < RAIL_MIN {
        return String::new();
    }
    let mut out = String::from(
        "<nav class=\"outline\" aria-label=\"On this page\">\n<p class=\"outline-label\">On this page</p>\n<ol>\n",
    );
    for (id, title) in items {
        let _ = writeln!(out, "<li><a href=\"#{id}\">{}</a></li>", esc(title));
    }
    out.push_str("</ol>\n</nav>\n");
    out
}

/// One page per `docs/internals/*.md`, sorted by name.
pub fn pages(root: &Path, home: &str, repo: &str, site: &str, demo: &str) -> Result<(Vec<Out>, Vec<String>)> {
    let dir = root.join(DIR);
    let mut stems = Vec::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {DIR}"))? {
        let p = entry?.path();
        if p.extension().is_some_and(|e| e == "md")
            && let Some(s) = p.file_stem().and_then(|s| s.to_str())
        {
            stems.push(s.to_owned());
        }
    }
    stems.sort();
    let mut outs = Vec::new();
    let mut paths = Vec::new();
    for stem in stems {
        let rel = format!("{DIR}/{stem}.md");
        let text =
            std::fs::read_to_string(root.join(&rel)).with_context(|| format!("reading {rel}"))?;
        let title = text
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .with_context(|| format!("{rel} has no title"))?;
        let path = page_path(&stem);
        paths.push(path.clone());
        let rail = outline(&h2s(&text));
        let (open, close, script) = if rail.is_empty() {
            ("", "", String::new())
        } else {
            (
                "<div class=\"docgrid\">\n",
                "</div>\n",
                format!("<script>{OUTLINE_JS}</script>\n"),
            )
        };
        outs.push(Out::file(
            format!("site/{path}index.html"),
            format!(
                "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
                 <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
                 <meta name=\"color-scheme\" content=\"light dark\">\n\
                 <title>{t} - {s}</title>\n<link rel=\"canonical\" href=\"{home}/{path}\">\n\
                 <link rel=\"icon\" type=\"image/svg+xml\" href=\"/assets/mark.svg\">\n\
                 <link rel=\"stylesheet\" href=\"/assets/site.css\">\n\
                 <script>{THEME_INIT_JS}</script>\n</head>\n<body>\n{bar}{open}<main id=\"main\">\n\
                 {b}<footer>\n<a href=\"{repo}/blob/main/{rel}\">Source of this page</a>\n{fleet}</footer>\n\
                 </main>\n{rail}{close}<script>{THEME_TOGGLE_JS}</script>\n{script}</body>\n</html>\n",
                t = esc(title),
                s = esc(site),
                home = esc(home),
                repo = esc(repo),
                bar = bar("/assets/", site, repo, demo),
                b = body(&text),
                fleet = fleet(),
            ),
        ));
    }
    Ok((outs, paths))
}
