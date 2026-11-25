//! Repository text is hostile: a branch, tag, file, author, subject, document or worktree
//! file that carries markup must reach every page as text. One crawl over a repository whose
//! every name and body holds the probe finds a page that writes it raw, whichever template
//! the `|safe` audit (`safe_sites.rs`) left a site in.
//!
//! Not on Windows: it refuses `<`, `>` and `"` in a file name.
#![cfg(not(windows))]
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use gitterrace_testkit::{Fixture, get_as, links, page};

/// Names the probe so a raw copy is found by `contains`, escaped or not.
const MARK: &str = "XSSPROBE";
const TAG: &str = "<i id=XSSPROBE>";
/// Ref names may not hold a space: the same two probes without one.
const REF_TAG: &str = "<i/id=XSSPROBE>";
const REF_BREAK: &str = "x\"onfocus=\"XSSPROBE";
const BREAK: &str = "x\" onfocus=\"XSSPROBE";

/// A raw copy of the probe in `html`: an element the page did not write, or an attribute the
/// value broke out into. A quote in text is harmless, so the breakout counts only inside a tag.
fn raw_probe(html: &str) -> Option<String> {
    for needle in ["<i id=XSSPROBE", "<i/id=XSSPROBE"] {
        if let Some(at) = html.find(needle) {
            return Some(format!(
                "element at {at}: {}",
                &html[at.saturating_sub(40)..(at + 30).min(html.len())]
            ));
        }
    }
    for needle in ["<script>XSSPROBE", "onerror=XSSPROBE", "javascript:XSSPROBE\""] {
        if let Some(at) = html.find(needle) {
            return Some(format!("{needle} at {at}"));
        }
    }
    for needle in ["\" onfocus=\"XSSPROBE", "\"onfocus=\"XSSPROBE"] {
        for (at, _) in html.match_indices(needle) {
            let head = &html[..at];
            if head.rfind('<') > head.rfind('>') {
                return Some(format!("breakout at {at}: {}", &html[at.saturating_sub(60)..at]));
            }
        }
    }
    None
}

#[test]
fn the_detector_finds_each_raw_form_and_spares_text() {
    assert!(raw_probe("<b><i id=XSSPROBE></b>").is_some());
    assert!(raw_probe("<a title=\"x\" onfocus=\"XSSPROBE\">").is_some());
    assert!(raw_probe("<script>XSSPROBE()</script>").is_some());
    assert!(raw_probe("<img src=x onerror=XSSPROBE>").is_some());
    assert!(raw_probe("<p>x\" onfocus=\"XSSPROBE</p>").is_none());
    assert!(
        raw_probe("<a title=\"x&quot; onfocus=&quot;XSSPROBE\">&lt;i id=XSSPROBE&gt;</a>")
            .is_none()
    );
}

fn hostile_repo(dir: &std::path::Path) -> std::path::PathBuf {
    let readme = format!(
        "# {TAG}\n\n{TAG}\n\n<script>XSSPROBE</script>\n\n<img src=x onerror=XSSPROBE>\n\n\
         [a](javascript:XSSPROBE) [b](x\" onfocus=\"XSSPROBE)\n\n```\n{TAG}\n```\n"
    );
    let cell =
        |kind: &str, extra: &str| format!(r#"{{"cell_type":"{kind}","source":"{TAG}"{extra}}}"#);
    let outputs = r#","outputs":[{"output_type":"stream","text":"<i id=XSSPROBE>"},
        {"output_type":"display_data","data":{"text/html":"<script>XSSPROBE</script>",
        "image/svg+xml":"<svg onload=XSSPROBE>"}}]"#;
    let notebook = format!(r#"{{"cells":[{},{}]}}"#, cell("markdown", ""), cell("code", outputs));
    let f = Fixture::init(dir)
        .write("README.md", &readme)
        .write("LICENSE", &format!("{TAG}\n"))
        .write(&format!("{TAG}/a.rs"), &format!("fn {TAG}() {{ \"{BREAK}\" }}\n"))
        .write(&format!("{BREAK}.txt"), &format!("{TAG}\n"))
        .write("n.ipynb", &notebook)
        .write("t.csv", &format!("a,{TAG}\n{TAG},{BREAK}\n"))
        .write("p.svg", &format!("<svg onload=XSSPROBE>{TAG}</svg>"))
        .write("m.md", &format!("{TAG}\n\n| a |\n|---|\n| {TAG} |\n"))
        .at("2024-03-01T10:00:00Z")
        .commit(&format!("feat: {TAG} {BREAK}"))
        .write(&format!("{TAG}/a.rs"), &format!("fn {TAG}() {{ \"{BREAK}!\" }}\n"))
        .commit(&format!("fix({TAG}): {BREAK}"));
    f.git(&["config", "user.name", BREAK]);
    let f = f.write("late.txt", "x\n").commit_by(&format!("{BREAK} <a@b.c>"), "feat: by name");
    f.git(&["branch", &format!("b{REF_TAG}{REF_BREAK}")]);
    f.git(&["tag", "-a", &format!("t{REF_TAG}{REF_BREAK}"), "-m", &format!("{TAG} {BREAK}")]);
    let linked = dir.parent().expect("parent").join("linked-wt");
    let f = f.worktree(&linked, &format!("b{REF_TAG}{REF_BREAK}"));
    // A change and a new file in the worktree, for the worktree page.
    std::fs::write(f.path().join("README.md"), format!("# {TAG}\nmore\n")).expect("edit");
    std::fs::write(f.path().join(format!("new{TAG}.txt")), format!("{TAG}\n")).expect("new");
    f.path().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn no_page_writes_repository_text_as_markup() {
    let dir = tempfile::tempdir().expect("tmp");
    let repo = hostile_repo(&dir.path().join("r"));
    let router = crate::common::ready_router(&repo).await;
    let base = gitterrace_testkit::base(&router).await;
    let mut seeds = vec![format!("{base}/")];
    for tail in [
        "/worktree",
        "/refs",
        "/search?q=XSSPROBE",
        "/search?q=XSSPROBE&kind=commits",
        "/blob/HEAD/n.ipynb",
        "/blob/HEAD/README.md",
        "/blob/HEAD/t.csv",
        "/blob/HEAD/p.svg",
        "/blame/HEAD/README.md",
        "/stats",
        "/stats/hotspots?period=all",
        "/stats/languages?period=all",
        "/stats/ownership?period=all",
        "/compare",
    ] {
        seeds.push(format!("{base}{tail}"));
    }
    let mut bad = Vec::new();
    let mut escaped = 0;
    let reached = crate::common::crawl(
        &router,
        "localhost",
        seeds,
        600,
        |b| {
            links(b)
                .into_iter()
                .filter(|l| {
                    !l.contains("/raw/") && !l.starts_with("/static/") && l.starts_with(&base)
                })
                .collect()
        },
        |u, s, body| {
            assert!(s < 500, "{u}: {s}");
            // JSON carries the text as data, escaped by the client that reads it.
            if u.contains("/api/") {
                return;
            }
            escaped += usize::from(
                body.contains("&lt;i id=XSSPROBE") || body.contains("&lt;i/id=XSSPROBE"),
            );
            if let Some(why) = raw_probe(body) {
                bad.push(format!("{u}: {why}"));
            }
        },
    )
    .await;
    assert!(reached > 30, "crawl too shallow: {reached}");
    assert!(escaped > 8, "the probe reached only {escaped} pages: the crawl proves nothing");
    let (_, wt) = page(&router, "/worktree").await;
    assert!(wt.contains(MARK), "the worktree page shows the probe");
    let (_, host) = get_as(&router, &format!("{base}/refs"), "localhost").await;
    assert!(host.contains("linked-wt"), "the refs page lists the linked worktree");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
