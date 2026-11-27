//! Every `|safe` in a template is reviewed: the list below names each site (template and
//! expression) with the Rust that escapes or sanitizes the value. A new `|safe`, or a changed
//! expression, fails here until someone traces it and adds a row. A row whose site is gone
//! fails too, so the list cannot go stale.
//!
//! The class this guards: markup written past askama's escaping. The second defect it catches
//! is a reviewed site whose expression is swapped for another value without a new review.
//! `hostile_markup.rs` is the rendered side: the same sites fed repository text.

#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::{collections::BTreeMap, fs, path::Path};

/// (template, expression, where Rust makes the value safe).
const SITES: &[(&str, &str, &str)] = &[
    ("_base.html", "b", "views::banner: fixed words, esc_text(branch), fmt::sha_link_at"),
    ("blame.html", "html", "routes/files.rs blame: every cell through fmt::esc"),
    ("blob.html", "f", "filetype/font.rs html: escaped facts and name"),
    ("blob.html", "h", "routes/files.rs: highlight.rs (escaped spans) or fmt::esc"),
    ("blob.html", "l", "filetype/archive.rs html: entry names escaped"),
    ("blob.html", "m", "markdown::render_doc (ammonia) or media::blob_html (escaped)"),
    ("blob.html", "outline_rail", "markdown/outline.rs outline_html: escaped labels"),
    ("commit.html", "diff", "views/diff.rs diff_html_with: escaped hunks and paths"),
    ("commit.html", "p", "fmt::sha_link"),
    ("commit.html", "sha", "fmt::sha_link"),
    ("commit.html", "tools", "diff_find::toolbar: fixed markup and a count"),
    ("commit_line.html", "self.sha", "fmt::sha_link"),
    ("compare.html", "r.head_sha", "fmt::sha_link"),
    ("compare.html", "r.plan", "review_panel::render: escaped fields"),
    ("dochead.html", "files", "seg.rs Seg through seg_control.html (escapes)"),
    ("dochead.html", "h.outline", "markdown::outline_menu: escaped labels"),
    ("dochead.html", "h.tools", "dochead.rs: a fixed button"),
    ("dochead.html", "v", "seg.rs Seg through seg_control.html (escapes)"),
    ("dochead.html", "views", "seg.rs Seg through seg_control.html (escapes)"),
    ("hotspots.html", "patterns", "patterns.rs section: askama template, escaped"),
    ("languages_body.html", "c.sha", "fmt::sha_link"),
    ("languages_body.html", "s.json", "languages/render.rs: serde_json with < & U+2028/9 escaped"),
    ("notebook.html", "html", "markdown/notebook.rs: sanitized markdown or highlighter output"),
    ("overview.html", "a.html", "about.rs card: render_doc (ammonia) or esc_text in a pre"),
    ("overview.html", "l.sha", "fmt::sha_link"),
    ("overview.html", "side", "overview_side: askama template, escaped"),
    ("pagehead.html", "self.toolbar", "pagehead.rs: each caller builds it from escaped parts"),
    ("pagehead.html", "self.tools", "pagehead.rs: each caller builds it from escaped parts"),
    ("patterns_section.html", "coupled", "coupling.rs: askama template, escaped"),
    ("refs.html", "w.commit", "fmt::sha_link"),
    ("search.html", "l.html", "search.rs: highlight and Line, each part through fmt::esc"),
    ("side.html", "spark", "spark.rs: SVG from numbers, askama template"),
    ("stats.html", "frequency", "frequency.rs section: askama template, escaped"),
    ("stats.html", "types", "commit_types.rs section: askama template, escaped"),
    ("toolbar.html", "fb.jump", "markdown::outline_menu: escaped labels"),
    ("tree.html", "a.html", "about.rs card: render_doc (ammonia) or esc_text in a pre"),
    ("worktree.html", "diff", "views/diff.rs diff_html_with: escaped hunks and paths"),
    ("worktree.html", "plan", "review_panel::render: escaped fields"),
    ("worktree.html", "tools", "diff_find::toolbar and review bar: fixed markup"),
];

/// `text` without its `{# ... #}` comments, which may name `|safe` in prose.
fn without_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("{#") {
        out.push_str(&rest[..i]);
        rest = rest[i..].split_once("#}").map_or("", |(_, r)| r);
    }
    out.push_str(rest);
    out
}

/// Every `{{ expr|safe }}` of `text`, whitespace trimmed.
fn safe_exprs(text: &str) -> Vec<String> {
    let text = without_comments(text);
    let mut out = Vec::new();
    for (at, _) in text.match_indices("safe") {
        let after = text[at + 4..].trim_start().trim_start_matches('-').trim_start();
        let before = text[..at].trim_end();
        let (Some(before), true) = (before.strip_suffix('|'), after.starts_with("}}")) else {
            continue;
        };
        let open = before.rfind("{{").map_or(0, |i| i + 2);
        out.push(before[open..].trim_start_matches('-').trim().to_owned());
    }
    out
}

/// Sites found in `files` (name, text), as sorted distinct (template, expression) pairs.
fn found(files: &[(String, String)]) -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = files
        .iter()
        .flat_map(|(n, t)| safe_exprs(t).into_iter().map(move |e| (n.clone(), e)))
        .collect();
    all.sort();
    all.dedup();
    all
}

/// The differences between the reviewed list and `found`: unreviewed sites, then stale rows.
fn drift(found: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (t, e) in found {
        if !SITES.iter().any(|s| s.0 == t && s.1 == e) {
            bad.push(format!("{t}: unreviewed `{e}|safe`: trace it to its Rust and add a row"));
        }
    }
    for (t, e, _) in SITES {
        if !found.iter().any(|f| f.0 == *t && f.1 == *e) {
            bad.push(format!("{t}: row `{e}` has no |safe site left: remove it"));
        }
    }
    bad
}

fn templates() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut files = Vec::new();
    for e in fs::read_dir(dir).expect("templates").flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        files.push((name, fs::read_to_string(e.path()).expect("read")));
    }
    files
}

#[test]
fn every_safe_site_is_a_reviewed_row() {
    let bad = drift(&found(&templates()));
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_planted_safe_and_a_swapped_expression_are_found() {
    let mut files = templates();
    files.push(("planted.html".into(), "<p>{{ note|safe }}</p>".into()));
    let bad = drift(&found(&files));
    assert_eq!(bad.len(), 1, "{bad:?}");
    assert!(bad[0].contains("planted.html"));

    let mut files = templates();
    let blame = files.iter_mut().find(|f| f.0 == "blame.html").expect("blame");
    blame.1 = blame.1.replace("html|safe", "title|safe");
    let bad = drift(&found(&files));
    assert_eq!(bad.len(), 2, "unreviewed and stale: {bad:?}");
}

#[test]
fn every_row_names_its_rust_source() {
    let mut seen = BTreeMap::new();
    for (t, e, why) in SITES {
        assert!(!why.trim().is_empty(), "{t} {e}: no source");
        assert!(seen.insert((t, e), why).is_none(), "{t} {e}: listed twice");
    }
}

/// The site generator's sites. Its input is the repository's own docs and constants, never a
/// served repository, so each value is markup the generator built.
const SITE_SITES: &[(&str, &str, &str)] = &[
    ("bar.html", "demo_icon", "xtask site/bar.rs: a constant icon"),
    ("bar.html", "docs_icon", "xtask site/bar.rs: a constant icon"),
    ("bar.html", "docs_menu", "xtask site/docnav.rs: docnav.html, titles escaped"),
    ("bar.html", "theme_button", "xtask site/bar.rs: a constant button"),
    ("docnav.html", "d.current", "xtask site/docnav.rs: a fixed attribute or empty"),
    ("docnav.html", "docs_icon", "xtask site/bar.rs: a constant icon"),
    ("landing.html", "handoff", "xtask site/landing.rs: pulldown-cmark of landing.md"),
    ("landing.html", "promises", "xtask site/landing.rs: pulldown-cmark of landing.md"),
    ("landing.html", "why", "xtask site/landing.rs: pulldown-cmark of landing.md"),
    ("page.html", "bar", "xtask site/bar.rs bar"),
    ("page.html", "docnav", "xtask site/docnav.rs"),
    ("page.html", "footer", "xtask site/footer.rs"),
    ("page.html", "head", "xtask site/agents.rs json_ld: serde_json with every < escaped"),
    ("page.html", "inner", "xtask site/docs.rs body: pulldown-cmark of repository docs"),
    ("page.html", "menus", "xtask site/frame.rs: built by the site's templates"),
    ("page.html", "pager", "xtask site/frame.rs: built by the site's templates"),
    ("page.html", "rail", "xtask site/frame.rs: built by the site's templates"),
    ("page.html", "s", "xtask site/frame.rs: the site's own script"),
    ("page.html", "theme_init", "xtask site/frame.rs: the hashed theme snippet"),
];

#[test]
fn every_site_generator_safe_site_is_a_reviewed_row() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xtask/templates");
    let mut files = Vec::new();
    for e in fs::read_dir(dir).expect("xtask templates").flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if Path::new(&name).extension().is_some_and(|x| x == "html") {
            files.push((name, fs::read_to_string(e.path()).expect("read")));
        }
    }
    let got = found(&files);
    let mut bad = Vec::new();
    for (t, e) in &got {
        if !SITE_SITES.iter().any(|s| s.0 == t && s.1 == e) {
            bad.push(format!("xtask {t}: unreviewed `{e}|safe`"));
        }
    }
    for (t, e, _) in SITE_SITES {
        if !got.iter().any(|f| f.0 == *t && f.1 == *e) {
            bad.push(format!("xtask {t}: row `{e}` has no |safe site left"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
