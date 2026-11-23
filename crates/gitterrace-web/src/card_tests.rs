use gitterrace_testkit::Fixture;

use super::*;

fn repo(dir: &std::path::Path) -> std::path::PathBuf {
    Fixture::init(dir)
        .write("src/a.rs", "fn a() {}\n")
        .write("b.txt", "b\n")
        .commit("feat: one")
        .write("src/a.rs", "fn a() { 1 }\n")
        .commit("feat: two")
        .path()
        .to_owned()
}

fn linked(p: &std::path::Path, ready: bool) -> axum::Router {
    let app = if ready { crate::tests::ready_app(p) } else { crate::tests::new_app(p) };
    app.set_api(gitterrace_api::ApiState::open(p).expect("api"));
    crate::routes::router(app)
}

#[tokio::test]
async fn a_file_answers_a_card_with_a_link_into_the_repo() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path());
    let r = linked(&p, true);
    let (st, body) = gitterrace_testkit::get(&r, "/api/insight/HEAD/src/a.rs").await;
    assert_eq!(st, 200);
    assert!(body.contains("fcard-body"), "{body}");
    assert!(body.contains("2 changes"), "{body}");
    assert!(body.contains("/stats/hotspots?"), "{body}");
}

#[tokio::test]
async fn a_cold_index_and_a_missing_path_answer_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path());
    let (st, body) =
        gitterrace_testkit::get(&linked(&p, false), "/api/insight/HEAD/src/a.rs").await;
    assert_eq!((st, body.as_str()), (200, ""));
    let (st, body) = gitterrace_testkit::get(&linked(&p, true), "/api/insight/HEAD/gone.rs").await;
    assert_eq!((st, body.as_str()), (200, ""));
}

#[tokio::test]
async fn a_folder_answers_badges_for_its_hot_entries() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path());
    let r = linked(&p, true);
    let (st, body) = gitterrace_testkit::get(&r, "/api/insight/HEAD/src").await;
    assert_eq!(st, 200);
    assert!(body.contains("data-name=\"a.rs\""), "{body}");
}

#[tokio::test]
async fn blob_tree_and_history_pages_name_their_fragment() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = repo(dir.path());
    let r = linked(&p, true);
    for (page, want) in [
        ("/blob/HEAD/src/a.rs", "data-card=\"/api/insight/HEAD/src/a.rs\""),
        ("/tree/HEAD/src", "data-card=\"/api/insight/HEAD/src\""),
        ("/history/src/a.rs", "data-card=\"/api/insight/HEAD/src/a.rs\""),
    ] {
        let (st, body) = gitterrace_testkit::get(&r, page).await;
        assert_eq!(st, 200, "{page}");
        assert!(body.contains(want), "{page}: {want}");
    }
}

#[test]
fn age_reads_in_the_largest_whole_unit() {
    assert_eq!(age_words(3 * gitterrace_git::DAY), "3 days");
    assert_eq!(age_words(90 * gitterrace_git::DAY), "3 months");
    assert_eq!(age_words(800 * gitterrace_git::DAY), "2 years");
}
