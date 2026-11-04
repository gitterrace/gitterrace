//! Tests for the oracle's tie detection and digest.

use crate::oracle::{canonical, digest, tie_only};

fn row(time: &str, who: &str) -> String {
    format!("<li><b>{who}</b> <time data-tip=\"{time}\" tabindex=\"0\">2 months ago</time></li>")
}

#[test]
fn a_swap_within_equal_times_is_tie_only() {
    let a = [
        "<ul>".to_owned(),
        row("2024-01-01 12:00", "a"),
        row("2024-01-01 12:00", "b"),
        row("2023-01-01 12:00", "c"),
        "</ul>".to_owned(),
    ];
    let b = [
        "<ul>".to_owned(),
        row("2024-01-01 12:00", "b"),
        row("2024-01-01 12:00", "a"),
        row("2023-01-01 12:00", "c"),
        "</ul>".to_owned(),
    ];
    assert!(tie_only(&a.join("\n"), &b.join("\n")));
}

/// The red proof: two rows of different times swapped, a changed row, a changed non-row
/// line, and an added row all fail.
#[test]
fn anything_but_a_tie_reorder_is_a_real_difference() {
    let base = [
        "<ul>".to_owned(),
        row("2024-01-01 12:00", "a"),
        row("2023-01-01 12:00", "c"),
        "</ul>".to_owned(),
    ];
    let old = base.join("\n");
    let swapped = [
        "<ul>".to_owned(),
        base[2].clone(),
        base[1].clone(),
        "</ul>".to_owned(),
    ];
    assert!(!tie_only(&old, &swapped.join("\n")), "cross-time swap");
    let changed = [
        "<ul>".to_owned(),
        row("2024-01-01 12:00", "z"),
        base[2].clone(),
        "</ul>".to_owned(),
    ];
    assert!(!tie_only(&old, &changed.join("\n")), "changed row");
    assert!(
        !tie_only(&old, &old.replace("<ul>", "<ol>")),
        "changed markup"
    );
    assert!(
        !tie_only(&old, &format!("{old}\n{}", row("2022-01-01 12:00", "d"))),
        "added row"
    );
    assert!(!tie_only(&old, &old), "no change is not a tie reorder");
}

/// Rows on one line, as the commit list renders them: the same rules hold per row.
#[test]
fn rows_on_one_line_compare_row_by_row() {
    let one = |rows: &[String]| format!("<ul>{}</ul>", rows.concat());
    let (a, b, c) = (
        row("2024-01-01 12:00", "a"),
        row("2024-01-01 12:00", "b"),
        row("2023-01-01 12:00", "c"),
    );
    let old = one(&[a.clone(), b.clone(), c.clone()]);
    assert!(tie_only(&old, &one(&[b.clone(), a.clone(), c.clone()])));
    assert!(
        !tie_only(&old, &one(&[a, c, b])),
        "cross-time swap on one line"
    );
}

/// The golden's canonical digest accepts exactly what `tie_only` accepts: equal-time swaps.
#[test]
fn canonical_form_folds_ties_and_nothing_else() {
    let one = |rows: &[String]| format!("<ul>{}</ul>", rows.concat());
    let (a, b, c) = (
        row("2024-01-01 12:00", "a"),
        row("2024-01-01 12:00", "b"),
        row("2023-01-01 12:00", "c"),
    );
    let old = one(&[a.clone(), b.clone(), c.clone()]);
    assert_eq!(
        canonical(&old),
        canonical(&one(&[b.clone(), a.clone(), c.clone()]))
    );
    assert_ne!(canonical(&old), canonical(&one(&[a.clone(), c, b.clone()])));
    assert_ne!(
        canonical(&old),
        canonical(&one(&[a, b, row("2023-01-01 12:00", "z")]))
    );
}

/// Header changes on any page, and the sidebar on `/`, are chrome. Anything in the body of a
/// stats page, or the sidebar of a stats page, is not.
#[test]
fn only_header_and_overview_sidebar_are_chrome() {
    use crate::oracle::without_chrome;
    let page = |head: &str, side: &str, main: &str| {
        format!(
            "<html><header class=\"top\">{head}</header><main>{main}</main><aside>{side}</aside></html>"
        )
    };
    let (a, b) = (page("menu", "s1", "42"), page("menu2", "s2", "42"));
    assert_eq!(
        without_chrome("[as-is] /?period=1m", &a),
        without_chrome("[as-is] /?period=1m", &b)
    );
    assert_ne!(
        without_chrome("[as-is] /stats?period=1m", &a),
        without_chrome("[as-is] /stats?period=1m", &b)
    );
    let c = page("menu2", "s1", "43");
    assert_ne!(
        without_chrome("/?x", &a),
        without_chrome("/?x", &c),
        "planted body change"
    );
    let d = page("menu2", "s1", "42");
    assert_eq!(
        without_chrome("/stats/types?x", &a),
        without_chrome("/stats/types?x", &d)
    );
}

#[test]
fn digest_is_pinned() {
    assert_eq!(digest(""), "6c62272e07bb014262b821756295c58d");
    assert_ne!(digest("a"), digest("b"));
}

/// The re-bless guard: one real body change planted in one cell of a dump must come out of the
/// old vs new proof as REAL, with its line. The completeness proof, which renders one commit,
/// cannot see it, and a header-only or tie-only move must not be called real.
#[test]
fn a_planted_change_in_one_cell_is_real() {
    use crate::oracle::{Change, read_dump};
    let page = |head: &str, n: &str| {
        format!("<html><header>{head}</header>\n<main>\n<p>{n} commits</p>\n</main></html>")
    };
    let write = |dir: &std::path::Path, cells: &[(&str, String)]| {
        for (i, (k, b)) in cells.iter().enumerate() {
            std::fs::write(dir.join(format!("{i}")), format!("{k}\n{b}")).expect("dump");
        }
    };
    let base = std::env::temp_dir().join(format!("gt-change-{}", std::process::id()));
    let (old, new) = (base.join("old"), base.join("new"));
    std::fs::create_dir_all(&old).expect("mkdir");
    std::fs::create_dir_all(&new).expect("mkdir");
    write(
        &old,
        &[
            ("[as-is] /stats?period=1m", page("a", "42")),
            ("[as-is] /stats?period=1w", page("a", "7")),
            ("[as-is] /stats?period=1y", page("a", "90")),
        ],
    );
    write(
        &new,
        &[
            ("[as-is] /stats?period=1m", page("b", "42")),
            ("[as-is] /stats?period=1w", page("a", "8")),
            ("[as-is] /stats?period=1y", page("a", "90")),
        ],
    );
    let c = Change::of(&read_dump(&old), &read_dump(&new));
    std::fs::remove_dir_all(&base).expect("cleanup");
    assert_eq!((c.same, c.chrome.len(), c.ties.len()), (1, 1, 0));
    assert_eq!(c.real.len(), 1, "{}", c.render(""));
    let (k, was, now) = &c.real[0];
    assert_eq!(k, "[as-is] /stats?period=1w");
    assert_eq!(
        (was.as_str(), now.as_str()),
        ("<p>7 commits</p>", "<p>8 commits</p>")
    );
    assert!(c.render("").contains("REAL 1"));
    assert_eq!(c.unexplained(), 1);
}
