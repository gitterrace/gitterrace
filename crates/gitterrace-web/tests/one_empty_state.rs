//! Every list or page with nothing to show renders the one empty state `empty.rs` writes: the
//! message, centred and muted, with the next step as a link where there is one. A hand-written
//! "None" row or muted line drifts in spacing and offers no way on.

use std::path::Path;

use gitterrace_testkit::{Fixture, page};

/// Hand-written empty states in `code`: an element whose class names `empty`, or a muted line
/// or cell that opens with "No" or reads "None".
fn loose_empties(code: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for (i, _) in code.match_indices("class=") {
        let rest = &code[i + 6..];
        let rest = rest.trim_start_matches(['"', '\\']);
        let class = rest.split(['"', '\\']).next().unwrap_or_default();
        let after = rest
            .split_once('>')
            .map_or("", |(_, a)| a)
            .trim_start_matches(['"', '#']);
        // A hint a script shows while filtering a menu starts hidden; it is not a page state.
        let tag = rest.split('>').next().unwrap_or_default();
        if tag.contains(" hidden") {
            continue;
        }
        let words: Vec<&str> = class.split_whitespace().collect();
        let says_none = after.starts_with("No ") || after.starts_with("None<");
        if words.contains(&"empty") || (words.contains(&"muted") && says_none) {
            bad.push(code[i..].chars().take(80).collect());
        }
    }
    bad
}

#[test]
fn a_hand_written_empty_state_is_caught() {
    assert_eq!(loose_empties("<p class=\"muted\">No commits.</p>").len(), 1);
    assert_eq!(
        loose_empties("<tr><td class=\"muted\">None</td></tr>").len(),
        1
    );
    assert_eq!(
        loose_empties(r#"<p class="empty">No history yet.</p>"#).len(),
        1
    );
    assert_eq!(
        loose_empties("\"<p class=\\\"muted pad\\\">No textual changes.</p>\"").len(),
        1
    );
    assert!(loose_empties("<p class=\"muted\">Nothing else here</p>").is_empty());
    assert!(loose_empties("<span class=\"muted\">Notes</span>").is_empty());
    assert!(loose_empties("<p class=\"muted\" hidden>No matching heading</p>").is_empty());
}

/// The markup lives in `empty.rs` and `empty_state.html` and nowhere else in the crate's code,
/// templates or scripts.
#[test]
fn only_the_shared_empty_state_writes_one() {
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
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            // The owner and its markup; the not-found page, which is a whole page with its own
            // heading and several ways on; test files that read the markup back.
            if name.as_deref().is_some_and(|n| {
                n == "empty.rs"
                    || n == "empty_state.html"
                    || n == "lost.html"
                    || n.ends_with("tests.rs")
            }) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            for b in loose_empties(code) {
                bad.push(format!("{}: {b}", p.display()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "hand-written empty state:\n{}",
        bad.join("\n")
    );
}

/// Empty pages render the shared state with a way on, and no tab offers a page of nothing.
#[tokio::test]
async fn empty_pages_offer_a_next_step() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path())
        .write("a.rs", "fn main() {}\n")
        .commit("feat: first");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    let mut fails = Vec::new();
    for p in [
        "/log/HEAD?path=nope",
        "/history/nope.rs",
        "/stashes",
        "/worktrees",
        "/search?q=zzzqqq",
        "/stats?author=nobody%40example.com",
    ] {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        let Some(state) = body.split("<div class=\"empty\"").nth(1) else {
            fails.push(format!("{p}: no shared empty state"));
            continue;
        };
        let state = state.split("</div>").next().unwrap_or_default();
        if !state.contains("class=\"next\"><a href=") {
            fails.push(format!("{p}: no next step in {state}"));
        }
    }
    let (_, refs) = page(&router, "/refs").await;
    if refs.contains(">Stashes <") || refs.contains(">Worktrees <") {
        fails.push("/refs offers an empty tab".to_owned());
    }
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}
