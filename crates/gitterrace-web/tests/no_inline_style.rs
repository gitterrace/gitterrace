//! The app's own markup carries no `style` attribute: classes do that work, so a policy
//! stricter than the app's own (a host's, a proxy's) loses nothing. Renders the pages that
//! once carried one.

use gitterrace_testkit::git;

#[tokio::test(flavor = "multi_thread")]
async fn pages_carry_no_style_attribute() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(&p).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    let long = "line\n".repeat(250);
    std::fs::write(p.join("long.txt"), long).expect("w");
    std::fs::write(p.join("a.rs"), "fn main() {}\n").expect("w");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "feat(core): first"]);
    std::fs::write(p.join("a.rs"), "fn main() {\n}\n").expect("w");
    git(&p, &["commit", "-qam", "fix: second"]);

    let router = gitterrace_web::app_router(&p).expect("router");
    gitterrace_testkit::wait::page_until(&router, "/search?q=first&kind=commits", |b| {
        b.contains("class=\"crow\"")
    })
    .await;
    for page in [
        "/",
        "/log/HEAD",
        "/blob/HEAD/long.txt",
        "/stats/hotspots",
        "/stats/languages",
        "/stats/contributors",
        "/stats/contributors?metric=added",
        "/stats/contributors?metric=net",
    ] {
        let body = gitterrace_testkit::wait::page_until(&router, page, |b| {
            !page.starts_with("/stats") || gitterrace_testkit::wait::indexed(b)
        })
        .await;
        if page.contains("metric=") {
            assert!(body.contains("class=\"tsb\""), "{page} shows its bars");
        }
        assert!(!body.contains("style=\""), "{page} has an inline style");
    }
}

/// Every template, not only the pages above: a `style=` in any of them, including one behind
/// a condition no fixture reaches, fails here.
#[test]
fn no_template_has_a_style_attribute() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut bad = Vec::new();
    for e in std::fs::read_dir(&dir).expect("templates") {
        let path = e.expect("entry").path();
        let src = std::fs::read_to_string(&path).expect("read");
        for (n, line) in src.lines().enumerate() {
            if line.contains("style=") {
                bad.push(format!("{}:{}", path.display(), n + 1));
            }
        }
    }
    assert!(bad.is_empty(), "inline style in templates: {bad:?}");
}
