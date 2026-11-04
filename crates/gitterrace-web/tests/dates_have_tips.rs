//! Every relative date on every page carries its full date: the text sits inside
//! `<time data-tip datetime>`. The pages are the equivalence matrix's, plus the list pages
//! and the tree's last-commit column. Another guard keeps the `<time` markup in two places.

use std::path::Path;

use gitterrace_testkit::matrix::{self, Mailmap};
use gitterrace_testkit::{oracle, page};

/// Text a relative date can show.
const RELATIVE: &[&str] = &[
    "just now",
    " minute ago",
    " minutes ago",
    " hour ago",
    " hours ago",
    " day ago",
    " days ago",
    " month ago",
    " months ago",
    " year ago",
    " years ago",
];

/// Offsets of every relative-date text in `body` that is not the content of a complete
/// `<time data-tip=".." datetime="..">` element.
fn bare_dates(body: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for pat in RELATIVE {
        for (at, _) in body.match_indices(pat) {
            // Text starts after the last '>' before the match, with digits for "N unit ago".
            let start = body[..at].rfind('>').map_or(0, |i| i + 1);
            let text = &body[start..at + pat.len()];
            let head = &text[..text.len() - pat.len()];
            let ok_text = if *pat == "just now" {
                head.is_empty()
            } else {
                !head.is_empty() && head.chars().all(|c| c.is_ascii_digit())
            };
            let open = body[..start].rfind("<time ");
            let inside = ok_text
                && open.is_some_and(|o| {
                    let tag = &body[o..start];
                    tag.ends_with('>')
                        && tag.contains("data-tip=\"")
                        && tag.contains("datetime=\"")
                        && body[at + pat.len()..].starts_with("</time>")
                });
            if !inside {
                let from = body[..at]
                    .char_indices()
                    .rev()
                    .nth(60)
                    .map_or(0, |(i, _)| i);
                bad.push(body[from..at + pat.len()].to_owned());
            }
        }
    }
    bad
}

#[test]
fn the_checker_sees_a_bare_date() {
    assert_eq!(bare_dates("<span>3 days ago</span>").len(), 1);
    assert_eq!(
        bare_dates("<time tabindex=\"0\">3 days ago</time>").len(),
        1
    );
    let ok = "<time data-tip=\"x\" datetime=\"y\" tabindex=\"0\">3 days ago</time> \
              <time data-tip=\"x\" datetime=\"y\" tabindex=\"0\">just now</time>";
    assert!(bare_dates(ok).is_empty());
}

#[test]
fn every_relative_date_has_its_tip() {
    if !oracle::frozen("every_relative_date_has_its_tip") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let (repo, values) = oracle::matrix_repo(dir.path(), Mailmap::AsIs);
        let router = gitterrace_web::app_router(&repo).expect("router");
        oracle::ready(&router).await;
        let mut pages: Vec<String> = gitterrace_web::WARM_PAGES
            .iter()
            .map(|p| (*p).to_owned())
            .collect();
        pages.extend(
            [
                "/log/HEAD",
                "/refs",
                "/stashes",
                "/worktrees",
                "/tree/HEAD",
                "/tree/HEAD/src",
                "/blame/HEAD/src/lib.rs",
                "/history/src/lib.rs",
            ]
            .map(str::to_owned),
        );
        let mut seen = 0;
        let mut failures = Vec::new();
        for cell in matrix::pairwise()
            .iter()
            .filter(|c| c.mailmap == Mailmap::AsIs)
        {
            let q = cell.query(&values);
            for p in &pages {
                let (status, body) = page(&router, &format!("{p}?{q}")).await;
                assert_eq!(status, 200, "{p}");
                seen += body.matches("<time ").count();
                failures.extend(bare_dates(&body).into_iter().map(|b| format!("{p}: {b}")));
            }
        }
        assert!(seen > 0, "the matrix shows no dated element at all");
        // The tree's last-commit column is filled by JS from /api/last: every entry carries
        // the same When the server renders, and the helper turns it into a <time>.
        let (status, body) = page(&router, "/api/last/HEAD").await;
        assert_eq!(status, 200);
        let list: serde_json::Value = serde_json::from_str(&body).expect("json");
        for c in list.as_array().expect("array") {
            let w = &c["when"];
            assert!(w["text"].is_string(), "{c}");
            assert!(!w["datetime"].as_str().unwrap_or("").is_empty(), "{c}");
            assert!(!w["tip"].as_str().unwrap_or("").is_empty(), "{c}");
        }
        assert!(
            failures.is_empty(),
            "dates without a tip:\n{}",
            failures.join("\n")
        );
    });
}

/// The `<time` markup is written by `when.rs` on the server and `whenEl` in nav.js, nowhere else.
#[test]
fn only_the_shared_helpers_write_time_elements() {
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
                if p.file_name().is_some_and(|n| n != "vendor") {
                    stack.push(p);
                }
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            // when.rs writes the markup (its tests read it back), nav.js creates the element
            // once, in whenEl.
            let hand = match name.as_deref() {
                Some("when.rs") => false,
                Some("nav.js") => text.contains("<time") || text.matches("\"time\"").count() != 1,
                _ => {
                    text.contains("<time")
                        || p.extension().is_some_and(|x| x == "js") && text.contains("\"time\"")
                }
            };
            if hand {
                bad.push(p.display().to_string());
            }
        }
    }
    assert!(bad.is_empty(), "hand-written <time> in {bad:?}");
}
