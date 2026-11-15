//! The blob page shows a CSV as a table with its source a switch away.

use gitterrace_testkit::{Fixture, page};

#[tokio::test]
async fn a_csv_blob_renders_as_a_table() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path())
        .write("data.csv", "name,note\n\"Doe, J\",\"<i>x</i>\"\n")
        .commit("feat: data");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    let (status, body) = page(&router, "/blob/HEAD/data.csv").await;
    assert_eq!(status, 200);
    assert!(body.contains("class=\"data-table\""), "no table");
    assert!(body.contains("<td>Doe, J</td>"), "quoted comma split");
    assert!(!body.contains("<i>x</i>"), "cell unescaped");
    assert!(body.contains("data-pane=\"source\""), "no source pane");
}
