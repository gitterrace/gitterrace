//! Filter chips show what the visitor filtered on, escaped exactly once: a second pass
//! would print entities such as `&amp;` as text.

use gitterrace_testkit::git;

#[tokio::test(flavor = "multi_thread")]
async fn chip_labels_are_escaped_exactly_once() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(p.join("a&<b>")).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("a&<b>/x.rs"), "fn main() {}\n").expect("w");
    git(&p, &["add", "."]);
    git(
        &p,
        &[
            "-c",
            "user.name=Tom & <Jerry>",
            "-c",
            "user.email=t@x",
            "commit",
            "-q",
            "-m",
            "feat: first",
        ],
    );

    let router = gitterrace_web::app_router(&p).expect("router");
    let page = "/stats/contributors?author=t%40x&path=a%26%3Cb%3E";
    let body = gitterrace_testkit::wait::page_until(&router, page, |b| {
        b.contains("class=\"chip\"") && b.contains("Jerry")
    })
    .await;
    let chips = body
        .split("<div class=\"chips\">")
        .nth(1)
        .and_then(|s| s.split("</div>").next())
        .expect("chips");
    assert!(!chips.contains("amp;"), "{chips}");
    assert!(
        !chips.contains("<Jerry>") && !chips.contains("<b>"),
        "{chips}"
    );
}
