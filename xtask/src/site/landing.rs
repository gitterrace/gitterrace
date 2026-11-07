//! The landing page sections: install tabs, features, gallery, commands and the parts of
//! `docs/why.md` the page renders.

use std::fmt::Write as _;

use pulldown_cmark::{Options, Parser, html};

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
    out
}

/// The text before the first `##`, the body of "What it is not" style sections, and the
/// "How it is made" section, from `docs/why.md`.
pub(crate) fn split_why(why: &str) -> (&str, (String, String)) {
    let mut parts = why.split("\n## ");
    let head = parts.next().unwrap_or_default();
    let lead = head.split_once('\n').map_or("", |(_, rest)| rest);
    let (mut why_body, mut made) = (String::new(), String::new());
    for p in parts {
        let (title, body) = p.split_once('\n').unwrap_or((p, ""));
        if title.starts_with("How it is made") {
            made.push_str(body);
        } else {
            let _ = write!(why_body, "\n### {title}\n{body}\n");
        }
    }
    (lead, (why_body, made))
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
    html::push_html(&mut out, Parser::new_ext(text, Options::empty()));
    out
}
