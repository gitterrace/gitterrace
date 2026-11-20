//! Tests for a cache that goes bad while the server holds it: each must start a rebuild.

use gitterrace_testkit::git;

use crate::app::App;

#[test]
fn a_block_corrupted_while_serving_rebuilds_the_cache() {
    use gitterrace_index::feed::CommitRange;
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    std::fs::write(path.join("a.txt"), "1\n").expect("write");
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "feat: one"]);
    let repo = gitterrace_git::Repo::discover(path).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = App::new(repo, index);
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    app.mark_ready();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    let cache = app.index.dir().to_path_buf();
    let seg = std::fs::read_dir(&cache)
        .expect("ls")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.extension().is_some_and(|x| x == "gts")
                && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("seg-"))
        })
        .expect("a segment");
    // Zero every block byte (same length, so open-time checks still pass).
    let mut bytes = std::fs::read(&seg).expect("read");
    let end = bytes.len() - 64;
    let toc =
        usize::try_from(u64::from_le_bytes(bytes[end + 16..end + 24].try_into().expect("8 bytes")))
            .expect("toc");
    bytes[16..toc].fill(0);
    std::fs::write(&seg, &bytes).expect("corrupt");
    // A page reads the tables and the rows: the zeroed blocks fail their crc.
    let read = src.agg_author_day(i64::MIN..i64::MAX).count()
        + src.agg_path_author("").count()
        + src.rename_edges().count()
        + src.commits_in(CommitRange::All).count();
    assert_eq!(read, 0, "nothing of a corrupt block is handed out");
    drop(src);
    assert!(app.source(&head).expect("source").is_none(), "a corrupt set must not serve");
    assert!(!cache.join("MANIFEST").exists(), "the cache is deleted for a rebuild");
}

/// The class: the cache folder emptied or damaged from outside while the server holds the
/// index as ready. The second defect it catches is any ready state with no set to read
/// (a folder moved away, a manifest gone): each must start a rebuild, never answer "ready"
/// while every page shows the indexing page.
#[test]
fn a_cache_emptied_from_outside_while_ready_starts_a_rebuild() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    std::fs::write(path.join("a.txt"), "1\n").expect("write");
    git(path, &["add", "."]);
    git(path, &["commit", "-q", "-m", "feat: one"]);
    let repo = gitterrace_git::Repo::discover(path).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = App::new(repo, index);
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    app.mark_ready();
    let head = app.repo.resolve("HEAD").expect("head");
    let cache = app.index.dir().to_path_buf();
    for e in std::fs::read_dir(&cache).expect("ls").filter_map(Result::ok) {
        std::fs::remove_file(e.path()).expect("rm");
    }
    assert!(app.source(&head).expect("source").is_none());
    assert!(!app.ready(), "a ready index with no set to read rebuilds");
}
