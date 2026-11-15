//! The blob page shows a CSV as a table with its source a switch away, and an archive as
//! its entry list.

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

/// The blob page lists a tar.gz's entries without extracting them.
#[tokio::test]
async fn a_tar_gz_blob_lists_its_entries() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path());
    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut h = tar::Header::new_gnu();
    h.set_size(2);
    h.set_cksum();
    b.append_data(&mut h, "pkg/inner.txt", &b"hi"[..])
        .expect("append");
    let gz = b.into_inner().expect("tar").finish().expect("gz");
    std::fs::write(dir.path().join("bundle.tar.gz"), gz).expect("write");
    let f = f.commit("feat: bundle");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    let (status, body) = page(&router, "/blob/HEAD/bundle.tar.gz").await;
    assert_eq!(status, 200);
    assert!(body.contains("<td>pkg/inner.txt</td>"), "no listing");
}
