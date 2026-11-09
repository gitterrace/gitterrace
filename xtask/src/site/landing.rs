//! The landing page from `templates/landing.html`: the pitch, install, the quickstart, the
//! screenshots with two lines for what they cannot show, and the promises of `docs/why.md`
//! with a link to the rest of it. The page repeats nothing the Why page says beyond those
//! promises.

use askama::Template;
use pulldown_cmark::{Parser, html};

use crate::facts::Workspace;
use crate::readme_agents_md;
use crate::site::docs::Links;

/// The section of `docs/why.md` the landing page shows: its promises, which link to the
/// security model.
pub(crate) const SAFETY: &str = "What it does with your code";
/// The folder under `.git/` the cache lives in (`Index::open`). The files in it are
/// `CACHE_FILES`, so the prose may name only those.
const CACHE_DIR: &str = ".git/gitterrace/";

/// The screenshots, in gallery order (pairs side by side): file stem, alt text ("Caption: what
/// it shows"), caption. The site has a WebP pair per entry; the README shows the first, and
/// `docs/screenshots/` keeps only the PNG files the README links.
pub const SHOTS: [(&str, &str, &str); 6] = [
    (
        "overview",
        "Browse: the repository page with README, last commit, file list and key numbers.",
        "Browse",
    ),
    (
        "commit",
        "Commit: the message, the changed files and a highlighted diff.",
        "Commit",
    ),
    (
        "blame",
        "Blame: who last changed each line, with the commit and its age beside the code.",
        "Blame",
    ),
    (
        "send",
        "Send to: marked lines with their context, ready for mail, an issue, a merge request or \
         your task list.",
        "Send to",
    ),
    (
        "insights",
        "Hotspots: the files that change most, against how many people touch them.",
        "Hotspots",
    ),
    (
        "ownership",
        "Ownership: bus factor, solo and stale lines, and who holds each directory.",
        "Ownership",
    ),
];

/// What the screenshots cannot show, one line each: (title, docs page, docs section whose
/// first paragraph is the line). The text is the doc's own, so the landing page says nothing
/// the docs do not.
const LINES: [(&str, &str, &str); 2] = [
    ("Actions", "docs/actions.md", "Actions"),
    (
        "Many repositories",
        "docs/using.md",
        "Serving many repositories",
    ),
];

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing<'a> {
    name: &'a str,
    pitch: &'a str,
    demo: &'a str,
    install: Vec<(&'static str, String)>,
    bin: &'a str,
    requires: Vec<String>,
    quickstart: &'a str,
    shots: Vec<(&'a str, &'a str, &'a str, &'a str)>,
    lines: Vec<(&'a str, String, String)>,
    promises: &'a str,
}

/// The landing page's body, its h1 first. `read` reads a repository file.
pub(crate) fn body(
    ws: &Workspace,
    pitch: &str,
    links: &Links,
    read: &dyn Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    let pkg = ws
        .packages
        .get("gitterrace")
        .ok_or_else(|| anyhow::anyhow!("package gitterrace"))?;
    let why = read("docs/why.md")?;
    let promises = section(&why, SAFETY)
        .ok_or_else(|| anyhow::anyhow!("docs/why.md has no '## {SAFETY}' section"))?;
    check_facts(promises)?;
    let mut rows = Vec::new();
    for (title, doc, head) in LINES {
        let text = read(doc)?;
        let para = section(&text, head)
            .and_then(|s| s.split("\n\n").map(str::trim).find(|p| !p.is_empty()))
            .ok_or_else(|| anyhow::anyhow!("{doc} has no '## {head}' paragraph"))?;
        let path = crate::site::docs::page_path(crate::site::docs::stem(doc));
        rows.push((
            title,
            format!("{path}#{}", crate::site::docs::slug(head)),
            plain(para),
        ));
    }
    let shots = SHOTS
        .iter()
        .map(|(file, alt, cap)| {
            let what = alt.split_once(": ").map_or(*alt, |(_, rest)| rest);
            (*file, *alt, *cap, what)
        })
        .collect();
    let html = Landing {
        name: &pkg.name,
        pitch,
        demo: &ws.demo,
        install: readme_agents_md::install_lines(ws),
        bin: pkg.bins.first().map_or("", String::as_str),
        requires: requirements(ws),
        quickstart: readme_agents_md::QUICKSTART,
        shots,
        lines: rows,
        promises: &md(promises, links),
    }
    .render()?;
    Ok(format!("{html}\n"))
}

/// What the release ships for Windows and the Rust a `cargo install` needs, both from cargo
/// metadata. There is no git version to name: every read goes through gix, and git itself
/// only dispatches `git terrace`.
pub(crate) fn requirements(ws: &Workspace) -> Vec<String> {
    let mut parts = Vec::new();
    let windows = ws.targets.iter().any(|t| t.contains("windows"));
    if windows && !ws.windows_archive.is_empty() {
        parts.push(format!(
            "Windows: a {} from the release",
            ws.windows_archive.trim_start_matches('.')
        ));
    }
    if let Some(msrv) = ws
        .packages
        .get("gitterrace")
        .map(|p| p.rust_version.as_str())
        .filter(|v| !v.is_empty())
    {
        parts.push(format!("With cargo, Rust {msrv} or newer"));
    }
    parts
}

/// The body of the `## title` section of a markdown text, up to the next `## `.
pub(crate) fn section<'a>(text: &'a str, title: &str) -> Option<&'a str> {
    let head = format!("\n## {title}\n");
    let start = text.find(&head)? + head.len();
    let end = text[start..]
        .find("\n## ")
        .map_or(text.len(), |i| start + i);
    Some(&text[start..end])
}

/// One markdown paragraph as plain text for an escaped template value: links keep their
/// words, code keeps its text, line breaks become spaces.
fn plain(para: &str) -> String {
    let mut out = String::new();
    for ev in Parser::new_ext(para, gitterrace_web::markdown_options()) {
        match ev {
            pulldown_cmark::Event::Text(t) | pulldown_cmark::Event::Code(t) => out.push_str(&t),
            pulldown_cmark::Event::SoftBreak => out.push(' '),
            _ => {}
        }
    }
    out
}

/// Fails when the prose names a fact the code owns and gets it wrong: an `ip:port` in code that
/// is not the default address, or a path under `.git/` that is not the cache folder, a file the
/// cache lists, or the `info/exclude` line an export adds.
pub(crate) fn check_facts(text: &str) -> anyhow::Result<()> {
    let mut wrong = Vec::new();
    for span in text.split('`').skip(1).step_by(2) {
        let addr = span.rsplit_once(':').is_some_and(|(host, port)| {
            port.parse::<u16>().is_ok() && host.split('.').count() == 4
        });
        if addr && span != gitterrace_config::DEFAULT_ADDR {
            wrong.push(format!(
                "`{span}` is not the default address {}",
                gitterrace_config::DEFAULT_ADDR
            ));
        }
        if let Some(rest) = span.strip_prefix(".git/")
            && rest != "info/exclude"
        {
            let file = span.strip_prefix(CACHE_DIR);
            let known = file.is_some_and(|f| {
                f.is_empty() || gitterrace_index::CACHE_FILES.iter().any(|(n, _)| *n == f)
            });
            if !known {
                wrong.push(format!(
                    "`{span}` is not the cache folder {CACHE_DIR} or a file in it"
                ));
            }
        }
    }
    if wrong.is_empty() {
        Ok(())
    } else {
        anyhow::bail!("docs/why.md: {}", wrong.join("; "))
    }
}

/// Markdown from `docs/why.md` as HTML, with its relative links pointed at the site's pages,
/// as on the Why page itself.
fn md(text: &str, links: &Links) -> String {
    let mut out = String::new();
    let events = Parser::new_ext(text, gitterrace_web::markdown_options()).map(|ev| match ev {
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
            link_type,
            dest_url: links.rewrite("docs/why.md", &dest_url, false).into(),
            title,
            id,
        }),
        ev => ev,
    });
    html::push_html(&mut out, events);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The promises name the default address the code uses, and a wrong address, cache
    /// folder or cache file fails the build.
    #[test]
    fn why_prose_that_names_a_wrong_address_or_cache_path_fails() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/why.md");
        let why = std::fs::read_to_string(path).expect("why.md");
        let safety = section(&why, SAFETY).expect("safety section");
        check_facts(safety).expect("why.md agrees with the code");
        assert!(safety.contains(&format!("`{}`", gitterrace_config::DEFAULT_ADDR)));
        for bad in [
            "`127.0.0.1:8080`",
            "`.git/terrace/`",
            "`.git/gitterrace/blobs`",
        ] {
            let err = check_facts(&format!("serves on {bad}"))
                .expect_err(bad)
                .to_string();
            assert!(err.contains(bad), "{err}");
        }
        check_facts("`.git/gitterrace/`, `.git/info/exclude` and `127.0.0.1:4242`").expect("ok");
    }

    /// The requirements name the Windows archive and the MSRV from cargo metadata, so a bump
    /// in Cargo.toml shows on the page without an edit.
    #[test]
    fn requirements_come_from_cargo_metadata() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let ws = crate::facts::load(&root).expect("metadata");
        let msrv = &ws.packages["gitterrace"].rust_version;
        assert_eq!(
            requirements(&ws),
            [
                "Windows: a zip from the release".to_owned(),
                format!("With cargo, Rust {msrv} or newer")
            ]
        );
    }

    /// A relative doc link in the promises points at its page on the site.
    #[test]
    fn why_links_point_at_the_site_pages() {
        let links = Links::new("R");
        let s = md(
            "[a](online.md) [b](../SECURITY.md#x) [c](https://e.org)",
            &links,
        );
        assert!(s.contains("href=\"/docs/online/\""), "{s}");
        assert!(s.contains("href=\"R/blob/main/SECURITY.md#x\""), "{s}");
        assert!(s.contains("href=\"https://e.org\""), "{s}");
    }

    #[test]
    fn a_section_ends_at_the_next_heading_and_a_paragraph_reads_as_plain_text() {
        let text = "# T\n\n## A\n\nOne [link](x.md) and `code`\nwrapped.\n\nTwo.\n\n## B\n\nb\n";
        let a = section(text, "A").expect("A");
        assert!(!a.contains("## B") && a.contains("Two."), "{a:?}");
        assert_eq!(
            plain("One [link](x.md) and `code`\nwrapped."),
            "One link and code wrapped."
        );
        assert!(section(text, "C").is_none());
    }
}
