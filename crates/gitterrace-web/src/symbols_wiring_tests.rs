//! The file page lists a source file's symbols in the "On this page" outline, each a link to a
//! line the page shows.

use gitterrace_config::Config;
use gitterrace_testkit::{Fixture, page as get};

use crate::wiring_tests::{router, with};

const SOURCE: &str = "\
pub struct Point;

impl Point {
    pub fn new() -> Self { Point }
}

pub fn distance() -> f64 { 0.0 }
";

/// The `href`s of the outline links on a page, in order, from each `[data-outline]` block.
fn outline_links(body: &str) -> Vec<Vec<&str>> {
    body.split("data-outline")
        .skip(1)
        .map(|block| {
            let end = block.find("</ol>").unwrap_or(block.len());
            block[..end]
                .split("<a href=\"#")
                .skip(1)
                .filter_map(|a| a.split('"').next())
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn a_source_page_outlines_its_symbols_as_rail_and_menu_linked_to_shown_lines() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = Fixture::init(&dir.path().join("r"))
        .write("src/geo.rs", SOURCE)
        .write(
            "notes.txt",
            "fn not_code() {}\nfn still_not() {}\nfn nope() {}\n",
        )
        .commit("feat: geometry")
        .path()
        .to_owned();
    let (status, body) = get(&router(&p, Config::default()), "/blob/HEAD/src/geo.rs").await;
    assert_eq!(status, 200);
    let want = ["L1", "L3", "L4", "L7"];
    assert_eq!(outline_links(&body), [want, want], "rail and menu: {body}");
    for id in want {
        assert!(body.contains(&format!("id=\"{id}\"")), "{id} has no line");
    }
    assert!(body.contains("title=\"impl Point\""), "{body}");
    assert!(body.contains("src=\"/static/outline.js\""));

    // Lines past `limits.blob_lines` are not on the page, so their symbols are not listed.
    let cut = router(&p, with(|c| c.limits.blob_lines = 4));
    let (_, short) = get(&cut, "/blob/HEAD/src/geo.rs").await;
    let short_want = ["L1", "L3", "L4"];
    assert_eq!(outline_links(&short), [short_want, short_want]);

    let (_, text) = get(&router(&p, Config::default()), "/blob/HEAD/notes.txt").await;
    assert!(!text.contains("data-outline"), "plain text has no outline");
}

#[tokio::test]
async fn a_source_file_with_too_few_symbols_has_no_outline() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = Fixture::init(&dir.path().join("r"))
        .write("a.py", "def one():\n    pass\n\ndef two():\n    pass\n")
        .commit("feat: two")
        .path()
        .to_owned();
    let (_, body) = get(&router(&p, Config::default()), "/blob/HEAD/a.py").await;
    assert!(!body.contains("data-outline"), "two symbols: {body}");
    assert!(!body.contains("outline-cols"), "{body}");
}

/// A Rust and a Python file's outline names what each symbol is, in the rail and the menu.
#[tokio::test]
async fn a_source_page_outline_names_each_symbols_kind() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = Fixture::init(&dir.path().join("r"))
        .write(
            "src/geo.rs",
            "pub struct Point;\n\nimpl Point {\n    pub fn new() -> Self { Point }\n}\n",
        )
        .write(
            "tools/r.py",
            "class Report:\n    def total(self):\n        pass\n\ndef main():\n    pass\n",
        )
        .commit("feat: kinds")
        .path()
        .to_owned();
    let app = router(&p, Config::default());
    let (_, rs) = get(&app, "/blob/HEAD/src/geo.rs").await;
    for want in [
        ">struct</span><span class=\"vh\">struct </span><span class=\"oname\">Point<",
        ">impl</span><span class=\"vh\">implementation </span><span class=\"oname\">Point<",
        ">method</span><span class=\"vh\">method </span><span class=\"oname\">new<",
    ] {
        assert_eq!(rs.matches(want).count(), 2, "rail and menu: {want}\n{rs}");
    }
    assert!(rs.contains("title=\"impl Point\""), "{rs}");
    let (_, py) = get(&app, "/blob/HEAD/tools/r.py").await;
    for want in [
        ">class</span><span class=\"vh\">class </span><span class=\"oname\">Report<",
        ">method</span><span class=\"vh\">method </span><span class=\"oname\">total<",
        ">fn</span><span class=\"vh\">function </span><span class=\"oname\">main<",
    ] {
        assert_eq!(py.matches(want).count(), 2, "rail and menu: {want}\n{py}");
    }
}
