//! Every period, metric, axis, range and view switch is the one segmented control `seg.rs`
//! writes. A hand-written copy drifts in spacing, touch size and focus ring, which is how the
//! period choices once ended up glued together.

use std::path::Path;

use gitterrace_testkit::{Fixture, page};

/// Markup only `seg.rs` writes: the container, its parts, the old hand-written class and the
/// pressed state of an in-place switch.
const OWNED: &[&str] = &[
    "segctl",
    "class=\"segs",
    "segcap",
    "segsel",
    "data-seg-select",
    "class=\"seg\"",
    "class=\"seg ",
    "aria-pressed",
];

/// The markup of `code` that only the shared control may write.
fn owned_in(code: &str) -> Vec<&'static str> {
    OWNED.iter().copied().filter(|m| code.contains(m)).collect()
}

/// A run of two or more sibling links where one is marked `class="on"` and none sits in the
/// shared control: the shape of a hand-written switch, whatever its class is called.
fn loose_switches(body: &str) -> Vec<String> {
    let mut bad = Vec::new();
    // The tab row, its overflow menu included, is navigation, not a switch.
    let mut rest = body;
    let mut page = String::new();
    while let Some(i) = rest.find("<nav") {
        page.push_str(&rest[..i]);
        rest = rest[i..]
            .split_once("</nav>")
            .map_or("", |(_, after)| after);
    }
    page.push_str(rest);
    for div in page.split("<div").skip(1) {
        let open = div.split('>').next().unwrap_or_default();
        // Navigation tabs and the shared control are not switches written by hand; the option
        // toggles each flip one setting and are not one choice among several.
        if open.contains("segctl") || open.contains("opts") {
            continue;
        }
        let inner = div.split("</div>").next().unwrap_or_default();
        if inner.contains("<nav") || inner.contains("segctl") {
            continue;
        }
        let links = inner.matches("<a ").count();
        let on = inner.matches(" class=\"on\"").count();
        if links >= 2 && on == 1 && !inner.contains("<table") && !inner.contains("<li") {
            bad.push(div.chars().take(160).collect());
        }
    }
    bad
}

#[test]
fn a_hand_written_switch_is_caught() {
    // The old period picker, and a metric switch under any other class name.
    let period = "<div class=\"seg\"><span>Period</span><a href=\"a\" class=\"on\">Week</a><a href=\"b\">Month</a></div>";
    assert_eq!(owned_in(period), ["class=\"seg\""]);
    let metric =
        "<div class=\"pick\"><a href=\"a\" class=\"on\">Commits</a><a href=\"b\">Lines</a></div>";
    assert_eq!(loose_switches(metric).len(), 1);
    assert!(owned_in("<div class=\"chips\"></div>").is_empty());
    let shared = "<div class=\"segctl\" role=\"group\"><span class=\"segs\"><a class=\"on\">a</a><a>b</a></span></div>";
    assert!(loose_switches(shared).is_empty());
}

/// The markup lives in `seg.rs` and its `seg_control.html` and nowhere else in the crate's code, templates or scripts.
#[test]
fn only_the_shared_control_writes_a_switch() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    let mut stack = vec![
        root.join("src"),
        root.join("templates"),
        root.join("static"),
    ];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            // The owner, the stylesheet that styles it, the script that wires the narrow
            // select and the document views, and test files that read the markup back.
            if name.as_deref().is_some_and(|n| {
                n == "seg.rs"
                    || n == "seg_control.html"
                    || n == "app.css"
                    || n == "app.js"
                    || n == "markdown.js"
                    || n.ends_with("tests.rs")
            }) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            let found = owned_in(code);
            if !found.is_empty() {
                bad.push(format!("{}: {found:?}", p.display()));
            }
        }
    }
    assert!(bad.is_empty(), "hand-written switch in {bad:?}");
}

/// Every insights page and a document page render their switches through the shared control,
/// and none carries a switch shaped by hand.
#[tokio::test]
async fn insights_switches_are_the_shared_control() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path())
        .write("README.md", "# T\n\ntext\n")
        .write("src/a.rs", "fn main() {}\n")
        .commit("feat: first")
        .write("src/a.rs", "fn main() {\n}\n")
        .commit("fix: second");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    let mut fails = Vec::new();
    for (p, want) in [
        ("/stats", 1),
        ("/stats/contributors", 2),
        ("/stats/contributors?metric=added", 2),
        ("/stats/hotspots", 1),
        ("/stats/ownership", 1),
        ("/stats/languages", 3),
        ("/blob/HEAD/README.md", 1),
    ] {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        let got = body.matches("class=\"segctl\"").count();
        if got < want {
            fails.push(format!("{p}: {got} shared controls, want {want}"));
        }
        fails.extend(
            loose_switches(&body)
                .into_iter()
                .map(|b| format!("{p}: {b}")),
        );
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
