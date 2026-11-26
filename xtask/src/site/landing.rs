//! The landing page from `templates/landing.html`: the pitch, the why beside the overview,
//! install, the quickstart, one large screenshot with two lines for what it cannot show, and the
//! promises. Its prose lives in `docs/landing.md`, each sentence once, and no landing sentence
//! repeats on the page or in another doc.

use askama::Template;
use pulldown_cmark::{Parser, html};

use crate::facts::Workspace;
use crate::readme_agents_md;
use crate::site::docs::{Links, code_block, fences};

/// The landing page's words, one `##` section per block, each sentence here once.
pub(crate) const LANDING: &str = "docs/landing.md";
/// The section of the landing copy with the promises, which link to the security model.
pub(crate) const SAFETY: &str = "What it does with your code";
/// The section of the landing copy beside the overview picture.
const WHY: &str = "Why";
/// The folder under `.git/` the cache lives in (`Index::open`). The files in it are
/// `CACHE_FILES`, so the prose may name only those.
const CACHE_DIR: &str = ".git/gitterrace/";

/// The screenshots (the landing page shows the hero and `SHOWCASE`): file stem,
/// alt text ("Caption: what it shows"), caption. The site has a WebP pair per entry; the
/// README shows the first, and `docs/screenshots/` keeps only the PNG files the README links.
pub const SHOTS: [(&str, &str, &str); 8] = [
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
    (
        "blame",
        "Blame: who last changed each line, with the commit and its age beside the code.",
        "Blame",
    ),
    ("commit", "Commit: the message, the changed files and a highlighted diff.", "Commit"),
    ("narrow", "Phone: the commit page on a 390 px wide screen.", "Phone"),
    (
        "send",
        "Send to: marked lines with their context, ready for mail, an issue, a merge request or \
         your task list.",
        "Send to",
    ),
    (
        "agent",
        "Actions: a marked line offered to a coding agent beside the other targets.",
        "Agent",
    ),
    (
        "overview",
        "Browse: the repository page with README, last commit, file list and key numbers.",
        "Browse",
    ),
];

/// The one large screenshot on the landing page, shown full size.
pub(super) const SHOWCASE: &str = "insights";
pub(super) const HERO: &str = "overview";

/// The pixel size of a shot's thumbnail and of its full-size file. The phone crop is a 390 px
/// wide screen at 2x, the rest a 1280x800 window at 2x.
pub(crate) fn sizes(stem: &str) -> ((u32, u32), (u32, u32)) {
    if stem == "narrow" { ((390, 780), (780, 1560)) } else { ((1280, 800), (2560, 1600)) }
}

/// The alt text of the hero under the pitch: the repository page, which shows what it is.
const HERO_ALT: &str = "The repository page: README, last commit, file list and key numbers.";

/// What the screenshots cannot show, one line each: (title, docs page, docs section whose
/// first paragraph is the line). The text is the doc's own, so the landing page says nothing
/// the docs do not.
const LINES: [(&str, &str, &str); 2] = [
    ("Actions", "docs/actions.md", "Actions"),
    ("Many repositories", "docs/using.md", "Browse several repositories"),
];

/// Where a finding goes, shown as a row under the pitch so "any tool" reads at a glance:
/// (label, docs path with anchor). The site's link check fails on an anchor that is gone.
const DESTINATIONS: [(&str, &str); 6] = [
    ("Coding agents", "docs/actions/#hand-a-finding-to-an-agent"),
    ("Your editor", "docs/actions/#the-starter-file"),
    ("Issues and tasks", "docs/actions/#send-to"),
    ("Mail", "docs/actions/#send-to"),
    ("Scripts and webhooks", "docs/actions/#actions"),
    ("API and MCP", "docs/api/"),
];

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing<'a> {
    pitch: &'a str,
    install: Vec<(&'static str, String)>,
    bin: &'a str,
    requires: Vec<String>,
    quickstart: String,
    showcase: (&'a str, &'a str, &'a str, &'a str),
    hero: (&'a str, &'a str),
    hero_cmds: String,
    handoff: String,
    demo: &'a str,
    repo: &'a str,
    lines: Vec<(&'a str, String, String)>,
    promises: &'a str,
    story: &'a str,
    destinations: &'a [(&'a str, &'a str)],
}

/// The landing page's body, its h1 first. `read` reads a repository file.
pub(crate) fn body(
    ws: &Workspace,
    pitch: &str,
    links: &Links,
    read: &dyn Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    let pkg = ws.packages.get("gitterrace").ok_or_else(|| anyhow::anyhow!("package gitterrace"))?;
    let copy = read(LANDING)?;
    let promises = section(&copy, SAFETY)
        .ok_or_else(|| anyhow::anyhow!("{LANDING} has no '## {SAFETY}' section"))?;
    check_facts(promises)?;
    let story = section(&copy, WHY)
        .ok_or_else(|| anyhow::anyhow!("{LANDING} has no '## {WHY}' section"))?;
    let mut rows = Vec::new();
    for (title, doc, head) in LINES {
        let text = read(doc)?;
        let para = section(&text, head)
            .and_then(|s| s.split("\n\n").map(str::trim).find(|p| !p.is_empty()))
            .ok_or_else(|| anyhow::anyhow!("{doc} has no '## {head}' paragraph"))?;
        let para = lead(para).ok_or_else(|| {
            anyhow::anyhow!("{doc} '## {head}' opens with no sentence that stands alone")
        })?;
        let path = crate::site::docs::page_path(crate::site::docs::stem(doc));
        rows.push((title, format!("{path}#{}", crate::site::docs::slug(head)), plain(para)));
    }
    let (file, alt, cap) = SHOTS
        .iter()
        .find(|s| s.0 == SHOWCASE)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("no shot {SHOWCASE}"))?;
    let what = alt.split_once(": ").map_or(alt, |(_, rest)| rest);
    let handoff = section(&copy, "Hand-off")
        .ok_or_else(|| anyhow::anyhow!("{LANDING} has no '## Hand-off' section"))?;
    let install = readme_agents_md::install_lines(ws);
    let html = Landing {
        pitch,
        install: install
            .iter()
            .map(|(way, line)| (*way, code_block(&format!("{line}\n"), "bash")))
            .collect(),
        bin: pkg.bins.first().map_or("", String::as_str),
        requires: requirements(ws),
        quickstart: code_block(&readme_agents_md::quickstart(), "bash"),
        showcase: (file, alt, cap, what),
        hero: (HERO, HERO_ALT),
        hero_cmds: code_block(
            &format!("{}\ngit terrace\n", install.first().map_or("", |(_, l)| l.as_str())),
            "bash",
        ),
        handoff: md(handoff, links)?,
        demo: &ws.demo,
        repo: &pkg.repository,
        lines: rows,
        promises: &md(promises, links)?,
        story: &md(story, links)?,
        destinations: &DESTINATIONS,
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
    if let Some(msrv) =
        ws.packages.get("gitterrace").map(|p| p.rust_version.as_str()).filter(|v| !v.is_empty())
    {
        parts.push(format!("With cargo, Rust {msrv} or newer"));
    }
    parts
}

/// The body of the `## title` section of a markdown text, up to the next `## `.
pub(crate) fn section<'a>(text: &'a str, title: &str) -> Option<&'a str> {
    let head = format!("\n## {title}\n");
    let start = text.find(&head)? + head.len();
    let end = text[start..].find("\n## ").map_or(text.len(), |i| start + i);
    Some(&text[start..end])
}

/// The sentences of a paragraph up to the last one that ends in `.`, so a lead-in to a
/// code block ("...holds repositories:") never ends a landing line. None if there is none.
fn lead(para: &str) -> Option<&str> {
    let t = para.trim_end();
    if t.ends_with('.') {
        return Some(t);
    }
    // Markdown wraps lines, so a sentence end is ". " or ".\n".
    let end = t.rfind([' ', '\n']).and_then(|_| {
        t.char_indices()
            .rfind(|&(i, c)| c == '.' && t[i + 1..].starts_with([' ', '\n']))
            .map(|(i, _)| i + 1)
    })?;
    Some(&t[..end])
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
                wrong.push(format!("`{span}` is not the cache folder {CACHE_DIR} or a file in it"));
            }
        }
    }
    if wrong.is_empty() { Ok(()) } else { anyhow::bail!("{LANDING}: {}", wrong.join("; ")) }
}

/// Markdown from the landing copy as HTML, with its relative links pointed at the site's
/// pages, as from any doc.
fn md(text: &str, links: &Links) -> anyhow::Result<String> {
    let mut out = String::new();
    let events = Parser::new_ext(text, gitterrace_web::markdown_options()).map(|ev| match ev {
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link {
            link_type,
            dest_url: links.rewrite(LANDING, &dest_url, false).into(),
            title,
            id,
        }),
        ev => ev,
    });
    // The markers of a generated block are for the editor, not for the reader.
    let events = events.filter(|ev| {
        !matches!(ev, pulldown_cmark::Event::Html(h)
            if h.trim_start().starts_with("<!-- BEGIN GENERATED: ")
                || h.trim_start().starts_with("<!-- END GENERATED: "))
    });
    html::push_html(&mut out, fences(events.collect(), LANDING)?.into_iter());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The promises name the default address the code uses, and a wrong address, cache
    /// folder or cache file fails the build.
    #[test]
    fn a_landing_line_ends_on_a_full_stop_never_on_a_colon() {
        assert_eq!(lead("One. Two."), Some("One. Two."));
        assert_eq!(lead("One server. Give paths:"), Some("One server."));
        assert_eq!(lead("Give several paths:"), None);
    }

    #[test]
    fn landing_prose_that_names_a_wrong_address_or_cache_path_fails() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/landing.md");
        let copy = std::fs::read_to_string(path).expect("landing.md");
        let safety = section(&copy, SAFETY).expect("safety section");
        check_facts(safety).expect("landing.md agrees with the code");
        assert!(safety.contains(&format!("`{}`", gitterrace_config::DEFAULT_ADDR)));
        for bad in ["`127.0.0.1:8080`", "`.git/terrace/`", "`.git/gitterrace/blobs`"] {
            let err = check_facts(&format!("serves on {bad}")).expect_err(bad).to_string();
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
        let s = md("[a](online.md) [b](../SECURITY.md#x) [c](https://e.org)", &links).expect("md");
        assert!(s.contains("href=\"/docs/online/\""), "{s}");
        assert!(s.contains("href=\"R/blob/main/SECURITY.md#x\""), "{s}");
        assert!(s.contains("href=\"https://e.org\""), "{s}");
    }

    #[test]
    fn a_section_ends_at_the_next_heading_and_a_paragraph_reads_as_plain_text() {
        let text = "# T\n\n## A\n\nOne [link](x.md) and `code`\nwrapped.\n\nTwo.\n\n## B\n\nb\n";
        let a = section(text, "A").expect("A");
        assert!(!a.contains("## B") && a.contains("Two."), "{a:?}");
        assert_eq!(plain("One [link](x.md) and `code`\nwrapped."), "One link and code wrapped.");
        assert!(section(text, "C").is_none());
    }
}
