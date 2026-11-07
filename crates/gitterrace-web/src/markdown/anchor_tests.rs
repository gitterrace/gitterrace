//! Rendered heading and footnote anchors: plain where safe, prefixed and still paired where not.

use crate::markdown::{Base, outline_rail, render_doc};

fn doc(src: &str) -> crate::markdown::Doc {
    let base = Base {
        rev: "HEAD".to_owned(),
        dir: String::new(),
        highlight_bytes: crate::highlight::HIGHLIGHT_BYTES,
    };
    render_doc(src, base)
}

#[test]
fn heading_pilcrow_and_outline_hrefs_are_plain() {
    let d = doc("# Reference\n\n## Two\n\n## Three\n\n## Four\n\n[r](#reference)\n");
    assert!(
        d.html
            .contains("<h2 class=\"markdown-h1\" id=\"reference\""),
        "{}",
        d.html
    );
    assert!(
        d.html.contains("class=\"anchor\" href=\"#reference\""),
        "{}",
        d.html
    );
    assert!(d.html.contains("href=\"#reference\" rel"), "{}", d.html);
    let rail = outline_rail(&d.outline);
    assert!(rail.contains("<a href=\"#two\""), "{rail}");
    assert!(!d.html.contains("user-content-") && !rail.contains("user-content-"));
}

#[test]
fn colliding_heading_keeps_a_working_prefixed_target() {
    let d = doc("# main\n\n# q\n\n# location\n\n[m](#main)\n");
    for id in ["main", "q", "location"] {
        let pre = format!("user-content-{id}");
        assert!(
            d.html.contains(&format!("id=\"{pre}\"")),
            "{id}: {}",
            d.html
        );
        assert!(
            d.html
                .contains(&format!("class=\"anchor\" href=\"#{pre}\"")),
            "{id}"
        );
        assert!(
            !d.html.contains(&format!("id=\"{id}\"")),
            "{id} clobbers the page"
        );
        let rail = outline_rail(&d.outline);
        assert!(rail.contains(&format!("href=\"#{pre}\"")), "{rail}");
    }
    assert!(
        d.html.contains("<a href=\"#user-content-main\" rel"),
        "{}",
        d.html
    );
    let raw = doc("<div id=\"theme\">x</div>\n<span id=\"top\">y</span>\n");
    assert!(
        raw.html.contains("id=\"user-content-theme\""),
        "{}",
        raw.html
    );
    assert!(raw.html.contains("id=\"user-content-top\""), "{}", raw.html);
}

#[test]
fn footnote_refs_and_definitions_stay_paired() {
    let html = doc("note[^1] and[^b]\n\n[^1]: one\n[^b]: two\n").html;
    let ids: Vec<&str> = html
        .split("id=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    let hrefs: Vec<&str> = html
        .split("href=\"#")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .collect();
    assert!(!hrefs.is_empty(), "{html}");
    for h in &hrefs {
        assert!(ids.contains(h), "#{h} has no target in {html}");
    }
    assert!(!html.contains("user-content-"), "{html}");
}

#[test]
fn duplicate_headings_keep_their_suffixes() {
    let html = doc("## Same\n\n## Same\n\n## Same\n").html;
    for id in ["same", "same-1", "same-2"] {
        assert!(html.contains(&format!("id=\"{id}\"")), "{id}: {html}");
        assert!(html.contains(&format!("href=\"#{id}\"")), "{id}: {html}");
    }
}
