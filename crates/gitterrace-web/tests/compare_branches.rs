//! Compare reaches branches the served head does not. The index holds what the head reaches,
//! so a side branch's own commits are read from git: comparing `main` with it lists them and
//! counts their lines. An empty range says so and offers the other direction.

use gitterrace_testkit::{Fixture, oracle, page};

#[test]
fn a_side_branch_compares_and_an_empty_range_says_so() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tmp");
        let f = Fixture::init(dir.path())
            .at("2024-03-01T10:00:00Z")
            .write("a.rs", "fn a() {}\n")
            .commit("feat(core): start")
            .branch("side")
            .at("2024-03-02T10:00:00Z")
            .write("b.py", "x = 1\ny = 2\n")
            .commit("feat(core): side work")
            .checkout("main")
            .at("2024-03-03T10:00:00Z")
            .write("a.rs", "fn a() {}\nfn b() {}\n")
            .commit("fix(core): main work");
        let router = gitterrace_web::app_router(f.path()).expect("router");
        oracle::ready(&router).await;

        let (status, html) = page(&router, "/compare?base=main&head=side").await;
        assert_eq!(status, 200);
        assert!(html.contains("side work"), "the side commit is listed");
        assert!(
            html.contains("<b>1</b><span>Commits without merges"),
            "one commit counted"
        );
        assert!(html.contains("<td>Python</td>"), "its language is counted");

        let (_, same) = page(&router, "/compare?base=main&head=main").await;
        assert!(
            same.contains("class=\"empty\""),
            "an empty range shows the empty state"
        );
        assert!(same.contains("has no commits that"), "and says why");

        // Without a tag to start from, the page still says what to do and offers branches.
        let (_, start) = page(&router, "/compare?head=main").await;
        assert!(
            start.contains("Choose a base"),
            "an untagged start explains itself"
        );
        assert!(
            start.contains("compare?base=side&#38;head=main"),
            "and links the other branch"
        );
    });
}
