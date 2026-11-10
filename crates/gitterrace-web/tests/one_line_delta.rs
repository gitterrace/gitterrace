//! Every `+a -d` pair of lines added and removed is the one `_delta.html` macro or the
//! `delta.rs` helper writes. A hand-written pair drifts in its spacing, sign or grouping.

use std::path::Path;

use gitterrace_testkit::scan;

/// The join between the two halves of a pair, in a template or a Rust string.
const OWNED: &[&str] = &["</b> <b class=\"del\">", "</b> <b class=\\\"del\\\">"];

/// The hand-written pairs in `code`. Rust test modules may spell the markup in asserts.
fn loose_deltas(code: &str) -> usize {
    let code = scan::product(code);
    OWNED.iter().map(|m| code.matches(m).count()).sum()
}

#[test]
fn a_hand_written_pair_is_caught() {
    let tpl = "<b class=\"add\">+{{ a }}</b> <b class=\"del\">-{{ d }}</b>";
    assert_eq!(loose_deltas(tpl), 1);
    assert_eq!(
        loose_deltas("\"<b class=\\\"add\\\">+{}</b> <b class=\\\"del\\\">-{}</b>\""),
        1
    );
    assert_eq!(
        loose_deltas("{% call deltas::delta(a, d) %}{% endcall %}"),
        0
    );
    // A table with added and removed in their own cells is not a pair.
    assert_eq!(
        loose_deltas("<b class=\"add\">+1</b></td><td><b class=\"del\">-2</b>"),
        0
    );
}

#[test]
fn only_the_shared_delta_writes_a_pair() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let all = scan::files_in(root, &["templates", "src", "static"], &["html", "rs", "js"]);
    let bad: Vec<String> = all
        .iter()
        .filter(|p| !p.ends_with("_delta.html") && !p.ends_with("delta.rs"))
        .filter(|p| loose_deltas(&std::fs::read_to_string(p).unwrap_or_default()) > 0)
        .map(|p| p.display().to_string())
        .collect();
    assert!(bad.is_empty(), "a line delta written by hand: {bad:?}");
}
