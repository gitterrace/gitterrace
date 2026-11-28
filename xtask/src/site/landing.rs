//! The landing page from `templates/landing.html`: the pitch broken over a step, the demo and
//! the install command on one row, the app in a window that turns through a few real views
//! (each a crop without the app's own bars), the hand-off beside its
//! Send to window, and the promises. Its prose lives in `docs/landing.md`, each sentence once,
//! and no landing sentence repeats on the page or in another doc.

use askama::Template;
use pulldown_cmark::{Parser, html};

use crate::facts::Workspace;
use crate::readme_agents_md;
use crate::site::docs::{Links, fences};

/// The landing page's words, one `##` section per block, each sentence here once.
pub(crate) const LANDING: &str = "docs/landing.md";
/// The section of the landing copy with the promises, which link to the security model.
pub(crate) const SAFETY: &str = "What it does with your code";
/// The folder under `.git/` the cache lives in (`Index::open`). The files in it are
/// `CACHE_FILES`, so the prose may name only those.
const CACHE_DIR: &str = ".git/gitterrace/";

/// The screenshots: file stem, alt text ("Caption: what it shows"), caption. The repository
/// has a WebP pair per entry, the README shows two, and the landing page shows `REEL`
/// and the crop. The phone shots stay for the README.
pub const SHOTS: [(&str, &str, &str); 9] = [
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
        "narrow-insights",
        "Hotspots on a phone: the key numbers and the chart on a 390 px wide screen.",
        "Hotspots on a phone",
    ),
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

/// The views the window on the landing page turns through, in order: one of each kind of page
/// (insights, a file, a change, the repository), so no view repeats another's proof. The
/// first loads with the page, the others when they come near. The page crops each one in CSS
/// (`.c-<stem>`) to the part below the app's own bars, so the window's bar is the only one.
pub(super) const REEL: [&str; 4] = ["insights", "blame", "commit", "overview"];
/// The shot the hand-off shows a part of: the Send to window.
pub(super) const CROPS: [&str; 1] = ["send"];

/// The pixel size of a shot's thumbnail and of its full-size file. A phone shot is a 390 px
/// wide screen at 2x, the rest a 1280x800 window at 2x.
pub(crate) fn sizes(stem: &str) -> ((u32, u32), (u32, u32)) {
    match stem {
        "narrow" => ((390, 780), (780, 1560)),
        "narrow-insights" => ((390, 600), (780, 1200)),
        _ => ((1280, 800), (2560, 1600)),
    }
}

/// Where a finding goes, as a ruled list beside the hand-off picture, so "any tool" reads at
/// a glance: (label, docs path with anchor, what the destination receives). The site's link
/// check fails on an anchor that is gone.
const DESTINATIONS: [(&str, &str, &str); 6] = [
    ("Coding agents", "docs/actions/#hand-a-finding-to-an-agent", "prompt"),
    ("Your editor", "docs/actions/#the-starter-file", "path:line"),
    ("Issues and tasks", "docs/actions/#send-to", "new issue"),
    ("Mail", "docs/actions/#send-to", "mailto:"),
    ("Scripts and webhooks", "docs/actions/#actions", "stdin"),
    ("API and MCP", "docs/api/", "json"),
];

#[derive(Template)]
#[template(path = "landing.html")]
struct Landing<'a> {
    /// The pitch's two headline lines, one per step.
    l1: &'a str,
    l2: &'a str,
    why: &'a str,
    demo: &'a str,
    demo_note: &'a str,
    /// The first install command, which Copy copies.
    cmd: &'a str,
    addr: &'a str,
    reel: Vec<(&'a str, &'a str, &'a str)>,
    handoff: &'a str,
    handoff_links: &'a [(&'a str, &'a str, &'a str)],
    send: (&'a str, &'a str),
    promises: &'a str,
}

/// The landing page's body, its h1 first. `read` reads a repository file.
pub(crate) fn body(
    ws: &Workspace,
    pitch: &str,
    links: &Links,
    read: &dyn Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<String> {
    let copy = read(LANDING)?;
    let part = |title: &str| {
        section(&copy, title)
            .ok_or_else(|| anyhow::anyhow!("{LANDING} has no '## {title}' section"))
    };
    let promises = part(SAFETY)?;
    check_facts(promises)?;
    let (l1, l2) = headline(pitch)?;
    let install = readme_agents_md::install_lines(ws);
    let reel = REEL.iter().copied().map(shot).collect::<anyhow::Result<Vec<_>>>()?;
    let pair = |stem: &str| shot(stem).map(|s| (s.0, s.1));
    let html = Landing {
        l1,
        l2: &l2,
        why: &md(part("Why")?, links)?,
        demo: &ws.demo,
        demo_note: &listed(ws.demo_repos.iter().map(|r| r.0.as_str())),
        cmd: install.first().map_or("", |(_, l)| l.as_str()),
        addr: gitterrace_config::DEFAULT_ADDR,
        reel,
        handoff: &md(opening(part("Hand-off")?), links)?,
        handoff_links: &DESTINATIONS,
        send: pair(CROPS[0])?,
        promises: &md(promises, links)?,
    }
    .render()?;
    Ok(format!("{html}\n"))
}

/// The entry of `SHOTS` with this stem.
fn shot(stem: &str) -> anyhow::Result<(&'static str, &'static str, &'static str)> {
    SHOTS.iter().find(|s| s.0 == stem).copied().ok_or_else(|| anyhow::anyhow!("no shot {stem}"))
}

/// The pitch's first clause as the headline's two lines, broken before "before": line one
/// stands on the upper step and line two, with its full stop, on the lower. The rest of the
/// pitch is what the hand-off story shows. A pitch of another form fails generation, since
/// the page is drawn around this break.
fn headline(pitch: &str) -> anyhow::Result<(&str, String)> {
    let clause = pitch.split([',', '.']).next().unwrap_or(pitch);
    let (l1, rest) = clause
        .split_once(" before ")
        .ok_or_else(|| anyhow::anyhow!("the pitch has no ' before ' to break the headline at"))?;
    Ok((l1, format!("before {rest}.")))
}

/// The first paragraph of a section. The hand-off story shows its lead beside a picture; the
/// examples under it stay in the copy, which the site also serves as the page's markdown.
fn opening(section: &str) -> &str {
    section.split("\n\n").map(str::trim).find(|p| !p.is_empty()).unwrap_or("")
}

/// Names as a phrase: "a", "a and b", "a, b and c".
fn listed<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        Some((last, _)) => (*last).to_owned(),
        None => String::new(),
    }
}

/// The body of the `## title` section of a markdown text, up to the next `## `.
pub(crate) fn section<'a>(text: &'a str, title: &str) -> Option<&'a str> {
    let head = format!("\n## {title}\n");
    let start = text.find(&head)? + head.len();
    let end = text[start..].find("\n## ").map_or(text.len(), |i| start + i);
    Some(&text[start..end])
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

    /// The headline is the pitch's first clause in two lines, and a pitch that cannot be
    /// broken that way stops generation instead of drawing a one-line step.
    #[test]
    fn the_headline_breaks_the_pitch_before_before() {
        let (l1, l2) = headline("Review your repo before you push, and hand it on.").expect("ok");
        assert_eq!((l1, l2.as_str()), ("Review your repo", "before you push."));
        assert!(headline("Browse any repository.").is_err());
    }

    /// The demo note names every repository the demo shows, however many there are.
    #[test]
    fn names_read_as_a_phrase() {
        assert_eq!(listed(["a", "b", "c"].into_iter()), "a, b and c");
        assert_eq!(listed(["a", "b"].into_iter()), "a and b");
        assert_eq!(listed(["a"].into_iter()), "a");
        assert_eq!(listed(std::iter::empty()), "");
    }

    /// The hand-off story is its lead alone, and a fence in the copy still comes out
    /// highlighted, so a block the page shows later cannot bypass the shared handler.
    #[test]
    fn a_story_shows_its_first_paragraph_and_a_fence_is_highlighted() {
        let copy = "\nLead one.\nStill lead.\n\n```toml\na = 1\n```\n";
        assert_eq!(opening(copy), "Lead one.\nStill lead.");
        assert_eq!(opening("\n"), "");
        let s = md("```toml\na = 1\n```\n", &Links::new("R")).expect("md");
        assert!(s.contains("<pre>") && s.contains("class=\"s-"), "{s}");
    }

    /// Every shot the page shows is in `SHOTS`, so its files are checked and it has alt text.
    #[test]
    fn every_shot_the_page_shows_is_registered() {
        for stem in REEL.iter().chain(&CROPS) {
            shot(stem).expect("registered");
        }
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
    fn a_section_ends_at_the_next_heading() {
        let text = "# T\n\n## A\n\nOne.\n\nTwo.\n\n## B\n\nb\n";
        let a = section(text, "A").expect("A");
        assert!(!a.contains("## B") && a.contains("Two."), "{a:?}");
        assert!(section(text, "C").is_none());
    }
}
