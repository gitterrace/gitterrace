//! The file page shows a Jupyter notebook as its cells beside the source, and a notebook that
//! does not parse as source alone.

use std::sync::Arc;

use gitterrace_testkit::git;

use crate::{app::App, tests::get};

#[tokio::test]
async fn notebook_blob_renders_its_cells_beside_the_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    let nb = r##"{"cells":[{"cell_type":"markdown","source":"# Hi"},
        {"cell_type":"code","source":"x = 1","outputs":[{"output_type":"display_data",
        "data":{"text/html":"<script>bad()</script>"}}]}]}"##;
    std::fs::write(p.join("a.ipynb"), nb).expect("write");
    std::fs::write(p.join("b.ipynb"), "{broken").expect("write");
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "feat: nb"]);
    let repo = gitterrace_git::Repo::discover(p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = Arc::new(App::new(repo, index));
    let (s, body) = get(&app, "/blob/HEAD/a.ipynb").await;
    assert_eq!(s, 200);
    assert!(body.contains("class=\"nb-cell\""), "{body}");
    assert!(body.contains("data-pane=\"source\""));
    assert!(!body.contains("<script>bad"));
    let (s, body) = get(&app, "/blob/HEAD/b.ipynb").await;
    assert_eq!(s, 200);
    assert!(!body.contains("nb-cell"));
    assert!(body.contains("broken"), "{body}");
    assert!(body.contains("data-copyable"));
}
