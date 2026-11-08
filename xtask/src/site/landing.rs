//! The landing page sections: install tabs, features, gallery, commands and the parts of
//! `docs/why.md` the page renders.

use std::fmt::Write as _;

use pulldown_cmark::{Parser, html};

use crate::facts::Workspace;
use crate::readme_agents_md;
use crate::site::esc;

/// The install block as tabs, one per way to install. Radio inputs and CSS switch them, so the
/// page needs no script (the CSP allows only the hashed ones) and, with CSS off, every command
/// still shows. The first tab is open.
pub(crate) fn install_tabs(ws: &Workspace) -> String {
    let lines = readme_agents_md::install_lines(ws);
    let mut out = String::from("<div class=\"tabs\">\n");
    for (i, (way, _)) in lines.iter().enumerate() {
        let checked = if i == 0 { " checked" } else { "" };
        let _ = writeln!(
            out,
            "<input type=\"radio\" name=\"install\" id=\"install-{way}\" \
             class=\"tab-radio\"{checked}>\n\
             <label for=\"install-{way}\" class=\"tab-label\">{way}</label>"
        );
    }
    for (way, line) in &lines {
        let _ = writeln!(
            out,
            "<div class=\"tab-panel\" id=\"panel-{way}\"><pre><code>{}</code></pre></div>",
            esc(line)
        );
    }
    out.push_str("</div>\n");
    let names: Vec<String> = ws
        .packages
        .get("gitterrace")
        .map(|p| p.bins.iter().map(|b| format!("<code>{b}</code>")).collect())
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "<p>Installs {}; git runs it as <code>git terrace</code>.</p>",
        names.join(" and ")
    );
    out.push_str(&requirements(ws));
    out
}

/// One line under the tabs: what the release ships for Windows and the Rust a `cargo install`
/// needs, both from cargo metadata. There is no git version to name: every read goes through
/// gix, and git itself only dispatches `git terrace`.
pub(crate) fn requirements(ws: &Workspace) -> String {
    let mut parts = Vec::new();
    let windows = ws.targets.iter().any(|t| t.contains("windows"));
    if windows && !ws.windows_archive.is_empty() {
        parts.push(format!(
            "Windows: a <code>{}</code> from the release",
            esc(ws.windows_archive.trim_start_matches('.'))
        ));
    }
    if let Some(msrv) = ws
        .packages
        .get("gitterrace")
        .map(|p| p.rust_version.as_str())
        .filter(|v| !v.is_empty())
    {
        parts.push(format!("With cargo, Rust {} or newer", esc(msrv)));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("<p>{}.</p>\n", parts.join(". "))
}

/// The section of `docs/why.md` the page shows on its own, after the features.
pub(crate) const SAFETY: &str = "What it does with your code";
/// The section that closes the page, after the rest of `docs/why.md`.
pub(crate) const MADE: &str = "How it is made";
/// The section shown on its own after "Why": how gitterrace differs from tools a reader knows.
pub(crate) const COMPARED: &str = "Compared with";
/// The folder under `.git/` the cache lives in (`Index::open`). The files in it are
/// `CACHE_FILES`, so the prose may name only those.
const CACHE_DIR: &str = ".git/gitterrace/";

/// `docs/why.md` cut at its `##` headings: the lead, then `(title, body)` per section.
pub(crate) fn split_why(why: &str) -> (&str, Vec<(&str, &str)>) {
    let mut parts = why.split("\n## ");
    let head = parts.next().unwrap_or_default();
    let lead = head.split_once('\n').map_or("", |(_, rest)| rest);
    let sections = parts
        .map(|p| p.split_once('\n').unwrap_or((p, "")))
        .collect();
    (lead, sections)
}

/// The sections not shown on their own, as `###` subsections of "Why".
pub(crate) fn why_body(sections: &[(&str, &str)]) -> String {
    let mut s = String::new();
    for (title, body) in sections {
        if ![SAFETY, COMPARED, MADE].contains(title) {
            let _ = write!(s, "\n### {title}\n{body}\n");
        }
    }
    s
}

/// The body of one section, or an error naming it, so a renamed heading fails the build
/// instead of dropping the section from the page.
pub(crate) fn section<'a>(sections: &[(&str, &'a str)], title: &str) -> anyhow::Result<&'a str> {
    sections
        .iter()
        .find(|(t, _)| *t == title)
        .map(|(_, b)| *b)
        .ok_or_else(|| anyhow::anyhow!("docs/why.md has no '## {title}' section"))
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

/// Relative links in `docs/why.md` point at files beside it; on the page they point at the
/// repository, so `](online.md)` becomes `]({repo}/blob/main/docs/online.md)`.
pub(crate) fn site_links(text: &str, repo: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("](") {
        let (before, after) = rest.split_at(i + 2);
        out.push_str(before);
        let end = after.find(')').unwrap_or(after.len());
        let target = &after[..end];
        if target.contains("://") || target.starts_with('#') {
            out.push_str(target);
        } else {
            let path = target
                .strip_prefix("../")
                .map_or_else(|| format!("docs/{target}"), str::to_owned);
            let _ = write!(out, "{repo}/blob/main/{path}");
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// One row per feature.
pub(crate) fn features() -> String {
    let mut rows: Vec<(&str, String)> = Vec::new();
    let mut line = |head: &'static str, text: String| rows.push((head, text));
    let tabs: Vec<String> = gitterrace_web::stats_tab_labels()
        .iter()
        .map(|t| esc(&t.to_lowercase()))
        .collect();
    line("Insights.", format!("{}.", sentence(&tabs)));
    // Kinds of destination, not product names: the starter file names apps that may not be out
    // yet, and the page should not lead with one of them.
    line(
        "Send to.",
        "Hand a selection on with its context: a mail draft, an issue or merge request, a task \
         or a note."
            .to_owned(),
    );
    line(
        "Actions.",
        gitterrace_actions::schema::ACTION_SUMMARY.to_owned(),
    );
    line(
        "Many repositories.",
        "Run <code>git terrace</code> in another repository and it joins the server already \
         running: one address, a switcher between them, and every link stays valid."
            .to_owned(),
    );
    let mut s = String::new();
    for (head, text) in rows {
        let _ = writeln!(s, "<li><b>{head}</b> {text}</li>");
    }
    s
}

/// The shots as a two-column gallery of equal tiles, each with its caption.
pub(crate) fn gallery() -> String {
    let mut s = String::from("<div class=\"shots\">\n");
    for (name, alt, cap) in SHOTS {
        let _ = writeln!(
            s,
            "<figure>{}<figcaption><b>{cap}.</b> {}</figcaption></figure>",
            shot_html(name, alt, cap),
            esc(alt.split_once(": ").map_or(alt, |(_, rest)| rest))
        );
    }
    s.push_str("</div>\n");
    s
}

/// One row per command, the README block's rows, as a definition list so long text wraps
/// instead of running off the edge.
pub(crate) fn commands() -> String {
    let rows = readme_agents_md::commands_rows();
    let mut s = String::from("<dl class=\"cmds\">");
    for (a, b) in rows {
        let about = b.split(" [--").next().unwrap_or(&b);
        let _ = write!(
            s,
            "<dt><code>{}</code></dt><dd>{}</dd>",
            esc(&a),
            esc(about)
        );
    }
    s.push_str("</dl>\n");
    s
}

fn sentence(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{}, and {last}", head.join(", ")),
    }
}

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

/// One shot as a tile link plus a full-size view, for the caller's single `<figure>`. The view
/// is a `:target` overlay, so it works without script; clicking it (or Escape, through
/// [`LIGHTBOX_JS`](crate::site::LIGHTBOX_JS)) goes back to `#features`. Its image is a
/// decorative copy (empty alt) so the description is read once, and `tabindex="-1"` lets the
/// browser focus it when the fragment opens it.
pub(crate) fn shot_html(name: &str, alt: &str, cap: &str) -> String {
    // Every tile is the same size so the rows line up; check_shots holds the WebPs to it.
    // A tile shows only its thumbnail, so it never fetches the full-size file. Light and dark
    // are two lazy images, one hidden by the theme (the toggle or the system): a hidden lazy
    // image is never fetched, so only the shown scheme loads. The dark copy is decorative.
    let pair = |kind: &str, a: &str| {
        format!(
            "<span class=\"shot\"><img class=\"on-light\" \
             src=\"assets/screenshots/{name}-light{kind}.webp\" alt=\"{a}\" width=\"1280\" \
             height=\"800\" loading=\"lazy\" decoding=\"async\">\
             <img class=\"on-dark\" src=\"assets/screenshots/{name}-dark{kind}.webp\" alt=\"\" \
             width=\"1280\" height=\"800\" loading=\"lazy\" decoding=\"async\"></span>"
        )
    };
    let thumb = pair(".thumb", &esc(alt));
    // The lightbox loads only the full-size WebP for the colour scheme; its image is decorative.
    let full = pair("", "");
    format!(
        "<a href=\"#shot-{name}\">{}</a>\
         <a class=\"lightbox\" id=\"shot-{name}\" href=\"#features\" \
         aria-label=\"Close {}\" tabindex=\"-1\">{}</a>",
        thumb,
        esc(cap),
        full
    )
}

pub(crate) fn md(text: &str) -> String {
    let mut out = String::new();
    html::push_html(
        &mut out,
        Parser::new_ext(text, gitterrace_web::markdown_options()),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The safety section names the default address the code uses, and a wrong address, cache
    /// folder or cache file fails the build.
    #[test]
    fn why_prose_that_names_a_wrong_address_or_cache_path_fails() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/why.md");
        let why = std::fs::read_to_string(path).expect("why.md");
        let (_, sections) = split_why(&why);
        let safety = section(&sections, SAFETY).expect("safety section");
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

    /// The requirements line names the Windows archive and the MSRV from cargo metadata, so a
    /// bump in Cargo.toml shows on the page without an edit.
    #[test]
    fn requirements_come_from_cargo_metadata() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let ws = crate::facts::load(&root).expect("metadata");
        let msrv = &ws.packages["gitterrace"].rust_version;
        assert_eq!(
            requirements(&ws),
            format!(
                "<p>Windows: a <code>zip</code> from the release. \
                 With cargo, Rust {msrv} or newer.</p>\n"
            )
        );
    }

    /// A relative doc link on the landing page points at the file in the repository.
    #[test]
    fn why_links_point_at_the_repository() {
        let s = site_links(
            "[a](online.md) [b](../SECURITY.md#x) [c](https://e.org)",
            "R",
        );
        assert_eq!(
            s,
            "[a](R/blob/main/docs/online.md) [b](R/blob/main/SECURITY.md#x) [c](https://e.org)"
        );
    }
}
