//! The snapshot banner and the static-only stylesheet.

use anyhow::Context;

use crate::export::Project;

/// Inverse of the web's `fmt::enc`: `%XX` back to bytes, then UTF-8. Links are
/// matched and written by their real name, so `a b.md` exports as `a b.md`, not
/// `a%20b.md`, which a static server would decode and then miss.
/// Style for the snapshot banner and the static page, written next to the pages. Kept out of
/// the app's own CSS: only an export is ever static. Colours are the app's tokens, already on
/// the page. A static copy has no server, so the chrome that needs one is hidden: search, the
/// action buttons (their menu runs scripts and Send to), the working-tree chip and live notes.
/// Copy permalink stays: it needs nothing but the page.
pub(super) const SNAPSHOT_CSS: &str = ".snapshot{background:var(--accent-soft);color:var(--fg);\
 border-bottom:1px solid var(--line-strong);font:var(--text-sm)/1.4 var(--sans);padding:var(--space-2) 0}\
 .snapshot a{color:var(--accent-fg);font-weight:var(--weight-semibold)}\
 body[data-static] .topsearch,body[data-static] .actbtn,body[data-static] .wtchip,\
 body[data-static] .livenote,body[data-static] .actnote{display:none!important}\n";

/// What the snapshot banner names: the tag on HEAD when there is one, else the short commit id.
pub(super) fn snapshot_label(repo: &gitterrace_git::Repo, head: &str) -> anyhow::Result<String> {
    let tag = repo
        .refs()
        .context("reading refs")?
        .into_iter()
        .filter(|r| r.kind == gitterrace_git::RefKind::Tag && r.target == head)
        .map(|r| r.name)
        .max();
    Ok(tag.unwrap_or_else(|| gitterrace_git::short_id(head).to_owned()))
}

/// The banner every exported page carries, so nobody mistakes a frozen copy for a live view:
/// it says what it is, which revision it froze, and where the real thing comes from. Inserted
/// right after `<body ...>`, outside the app's chrome, so no template has to know about export.
pub(super) fn mark_snapshot(html: &[u8], label: &str, project: Option<&Project>) -> Vec<u8> {
    mark_snapshot_as(html, label, project, env!("GITTERRACE_VERSION"))
}

/// [`mark_snapshot`] naming `version`, escaped like every other value: it comes from the
/// build environment, which a test (or a packager) sets.
pub(super) fn mark_snapshot_as(
    html: &[u8],
    label: &str,
    project: Option<&Project>,
    version: &str,
) -> Vec<u8> {
    let version = gitterrace_types::esc(version);
    let page = String::from_utf8_lossy(html);
    // Anchor after `</head>`: a `<body` inside a title, a meta value or a script string in the
    // head is text, not the element.
    let from = page.find("</head>").unwrap_or(0);
    let Some(open) = ["<body ", "<body>"]
        .iter()
        .filter_map(|tag| page[from..].find(tag))
        .min()
        .map(|i| from + i)
    else {
        return html.to_vec();
    };
    let Some(end) = page[open..].find('>').map(|i| open + i + 1) else {
        return html.to_vec();
    };
    let what = match project {
        Some(p) => format!(
            "Static snapshot of <a href=\"{}\">{}</a> at <b>{}</b>, shown in gitterrace.",
            gitterrace_types::esc(&p.url),
            gitterrace_types::esc(&p.name),
            gitterrace_types::esc(label)
        ),
        // The index of several repositories names no revision of its own.
        None if label.is_empty() => "Static snapshot.".to_owned(),
        None => format!(
            "Static snapshot at <b>{}</b>.",
            gitterrace_types::esc(label)
        ),
    };
    let banner = format!(
        "<link rel=\"stylesheet\" href=\"/snapshot.css\">\
         <div class=\"snapshot\" role=\"note\"><div class=\"wrap\">{what} \
         Made with gitterrace {version}. \
         Search, live updates and actions need the real thing. \
         <a href=\"https://gitterrace.dev\">Install gitterrace</a></div></div>"
    );
    // The same version as a generator tag, so a copy of a single page still says what made it.
    let generator = format!("<meta name=\"generator\" content=\"gitterrace {version}\">");
    let head_end = page.find("</head>").unwrap_or(0);
    let mut out = String::with_capacity(page.len() + banner.len() + generator.len() + 12);
    // `data-static` tells the page scripts there is no server: they skip live updates and
    // the actions poll, and the snapshot CSS hides what needs one.
    out.push_str(&page[..head_end]);
    out.push_str(&generator);
    out.push_str(&page[head_end..end - 1]);
    out.push_str(" data-static>");
    out.push_str(&banner);
    out.push_str(&page[end..]);
    out.into_bytes()
}
