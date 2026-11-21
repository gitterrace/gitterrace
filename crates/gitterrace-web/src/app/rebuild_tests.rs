//! Tests for a cache that goes bad while the server holds it: each must start a rebuild.

#[test]
fn a_block_corrupted_while_serving_rebuilds_the_cache() {
    use gitterrace_index::feed::CommitRange;
    let (_dir, app, head) = crate::tests::one_commit_app();
    let src = crate::tests::unchecked_src(&app, &head);
    let cache = app.index.dir().to_path_buf();
    crate::tests::zero_blocks(&cache);
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
    let (_dir, app, head) = crate::tests::one_commit_app();
    let cache = app.index.dir().to_path_buf();
    for e in std::fs::read_dir(&cache).expect("ls").filter_map(Result::ok) {
        std::fs::remove_file(e.path()).expect("rm");
    }
    assert!(app.source(&head).expect("source").is_none());
    assert!(!app.ready(), "a ready index with no set to read rebuilds");
}
