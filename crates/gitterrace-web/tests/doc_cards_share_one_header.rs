//! Every card that shows a document opens with the one header `dochead.rs` writes, and nothing
//! else writes a card header: a copy would drift in look and lose the outline button.

use std::path::Path;

use gitterrace_testkit::{Fixture, page};

/// Markup only `dochead.rs` writes. A template or script holding one is a hand-written header.
const OWNED: &[&str] = &[
    "data-dochead",
    "class=\"dochead",
    "doctabs",
    "data-show=",
    "outlinebtn",
];

/// The card openings of `body`, each with the markup up to the next card.
fn cards(body: &str) -> Vec<&str> {
    let starts: Vec<usize> = body
        .match_indices("<div class=\"card")
        .chain(body.match_indices("<section class=\"card"))
        .map(|(i, _)| i)
        .collect();
    let mut starts = starts;
    starts.sort_unstable();
    starts
        .iter()
        .enumerate()
        .map(|(n, &s)| &body[s..starts.get(n + 1).copied().unwrap_or(body.len())])
        .collect()
}

/// What is wrong with the cards of `body`: a document card whose first child is not the shared
/// header, a card with two, or a card whose first child is classed like a header but is not one.
fn bad_cards(body: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for card in cards(body) {
        let Some(open_end) = card.find('>') else {
            continue;
        };
        let rest = card[open_end + 1..].trim_start();
        let first = rest.split('>').next().unwrap_or_default();
        let is_doc = card.contains("class=\"markdown-body") || card.contains("data-md-view");
        let heads = card.matches("data-dochead").count();
        let headlike = first.starts_with("<div")
            && first.split("class=\"").nth(1).is_some_and(|c| {
                let c = c.split('"').next().unwrap_or_default();
                c.split(' ')
                    .any(|w| w.ends_with("-h") || w.contains("head") || w == "tabs")
            });
        let shared = first.contains("data-dochead");
        if (is_doc && !shared) || heads > 1 || (headlike && !shared) {
            bad.push(card.chars().take(160).collect());
        }
    }
    bad
}

#[test]
fn a_hand_written_header_is_caught() {
    assert_eq!(
        bad_cards("<div class=\"card readme\"><div class=\"readme-h\">README</div><div class=\"markdown-body\"></div></div>").len(),
        1
    );
    assert_eq!(
        bad_cards("<div class=\"card\"><div class=\"doc-head\">x</div><pre></pre></div>").len(),
        1
    );
    assert!(bad_cards("<div class=\"card x\"><div class=\"dochead sticky\" data-dochead></div><div class=\"markdown-body\"></div></div>").is_empty());
}

/// The overview, a folder and a Markdown file, with one and with several about documents.
#[tokio::test]
async fn document_cards_open_with_the_shared_header() {
    let dir = tempfile::tempdir().expect("tmp");
    let doc = "# T\n\n## A\n\n## B\n\n## C\n";
    let f = Fixture::init(dir.path())
        .write("README.md", doc)
        .write("LICENSE", "MIT\n")
        .write("CONTRIBUTING.md", doc)
        .write("docs/README.md", doc)
        .write("src/a.rs", "fn main() {}\n")
        .commit("feat: docs");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    let mut fails = Vec::new();
    let mut seen = 0;
    for p in [
        "/",
        "/?doc=license",
        "/?doc=contributing",
        "/tree/HEAD",
        "/tree/HEAD/docs",
        "/blob/HEAD/README.md",
        "/blob/HEAD/CONTRIBUTING.md",
        "/blob/HEAD/src/a.rs",
        "/log/HEAD",
    ] {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        seen += body.matches("data-dochead").count();
        fails.extend(bad_cards(&body).into_iter().map(|b| format!("{p}: {b}")));
    }
    assert!(seen >= 7, "only {seen} shared headers");
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// The header's markup lives in `dochead.rs` and nowhere else in the crate.
#[test]
fn only_dochead_writes_card_headers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut bad = Vec::new();
    let mut stack = vec![root.join("src"), root.join("templates")];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            // The owner, the outline and the segmented control it embeds, and test files that
            // read the markup back.
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            if name.as_deref().is_some_and(|n| {
                n == "dochead.rs"
                    || n == "outline.rs"
                    || n == "seg.rs"
                    || n == "seg_control.html"
                    || n.ends_with("tests.rs")
            }) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            // Tests may read the markup back; only the code before them counts.
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            if OWNED.iter().any(|m| code.contains(m)) {
                bad.push(p.display().to_string());
            }
        }
    }
    assert!(bad.is_empty(), "hand-written card header in {bad:?}");
}
