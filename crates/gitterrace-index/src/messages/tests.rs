use std::path::Path;

use gitterrace_git::Repo;

use crate::Index;
use crate::seg::format::Kind;
use crate::seg::manifest::Manifest;

fn column(dir: &Path) -> Vec<Vec<u8>> {
    let m = Manifest::load(dir).expect("manifest").expect("set");
    m.live
        .iter()
        .filter(|f| f.kind == Kind::Messages as u8)
        .map(|f| std::fs::read(dir.join(&f.name)).expect("read"))
        .collect()
}

fn segments(dir: &Path) -> usize {
    Manifest::load(dir).ok().flatten().map_or(0, |m| {
        m.live
            .iter()
            .filter(|f| f.kind == Kind::Segment as u8)
            .count()
    })
}

/// Appends then the background merge: the merged column is byte for byte a fresh build's,
/// since the merge renumbers parents into the new canonical order.
#[test]
fn a_merged_column_equals_a_fresh_one_byte_for_byte() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f =
        gitterrace_testkit::Fixture::init(&dir.path().join("repo")).at("2024-01-01T00:00:00Z");
    let mut tips = Vec::new();
    for i in 0..12 {
        f = f
            .write(&format!("f{}", i % 3), &format!("{i}\n"))
            .commit(&format!("fix: step {i}"));
        tips.push(f.git(&["rev-parse", "HEAD"]));
    }
    let repo = Repo::discover(f.path()).expect("repo");
    let one = dir.path().join("one");
    let a = Index::open_dir(one.clone()).expect("index");
    a.set_message_index(true);
    a.update(&repo, tips.last().expect("tip"), &|_| {})
        .expect("one step");
    let want = column(&one);
    assert_eq!(want.len(), 1);

    let two = dir.path().join("two");
    let b = Index::open_dir(two.clone()).expect("index");
    b.set_message_index(true);
    // A fresh build then eight appends: nine segments, one more than starts the merge.
    for t in &tips[3..] {
        b.update(&repo, t, &|_| {}).expect("step");
    }
    gitterrace_testkit::wait::until_blocking("the merge ran", || segments(&two) == 1);
    assert_eq!(column(&two), want);
}

/// Bytes no writer produces: every flip and cut of a block is an error, never a panic or a
/// wrong item.
#[test]
fn hostile_items_error_not_panic() {
    use super::Item;
    use crate::seg::bounds::Cursor;
    let it = Item {
        time: -5,
        parents: vec![0, 1],
        text: Some((b"n".to_vec(), b"e".to_vec(), "msg".into())),
    };
    let mut raw = Vec::new();
    it.encode(&mut raw);
    let ok = Item::decode(&mut Cursor::new(&raw, "t", 0), 2).expect("round trip");
    assert_eq!(ok, it);
    assert!(
        Item::decode(&mut Cursor::new(&raw, "t", 0), 1).is_err(),
        "parent past the set"
    );
    for cut in 0..raw.len() {
        assert!(Item::decode(&mut Cursor::new(&raw[..cut], "t", 0), 2).is_err());
    }
    for i in 0..raw.len() {
        let mut b = raw.clone();
        b[i] ^= 0xff;
        drop(Item::decode(&mut Cursor::new(&b, "t", 0), 2));
    }
}

/// A message too large to keep is stored without its text and still found, from git.
#[test]
fn a_huge_message_is_kept_out_of_the_column_and_still_found() {
    use super::Item;
    let it = Item {
        time: 1,
        parents: Vec::new(),
        text: Some((Vec::new(), Vec::new(), "x".repeat(super::INLINE_MAX + 1))),
    };
    let mut raw = Vec::new();
    it.encode(&mut raw);
    assert!(raw.len() < 64);
    let back = Item::decode(&mut crate::seg::bounds::Cursor::new(&raw, "t", 0), 1).expect("decode");
    assert!(back.text.is_none());
}

/// Text big enough that every commit fills one block of its own.
fn big(i: usize) -> String {
    format!("fix: step {i}\n\n{}", "x".repeat(super::BLOCK_RAW))
}

fn history(dir: &Path, n: usize) -> (Repo, String) {
    let mut f = gitterrace_testkit::Fixture::init(&dir.join("repo")).at("2024-01-01T00:00:00Z");
    for i in 0..n {
        f = f.write("f", &format!("{i}\n")).commit(&big(i));
    }
    let tip = f.git(&["rev-parse", "HEAD"]);
    (Repo::discover(f.path()).expect("repo"), tip)
}

fn items(dir: &Path) -> Vec<super::Item> {
    let store = crate::seg::read::Store::open(dir)
        .expect("open")
        .expect("set");
    let col = super::MessageColumn::of(store)
        .expect("column")
        .expect("whole");
    col.items().expect("items")
}

/// A column longer than one file's toc goes on in the next file, and reads back exactly as
/// the one-file column. Real limit 4096 blocks: linux needs about 6000.
#[test]
fn a_column_past_a_full_toc_rolls_to_the_next_file_and_reads_back_equal() {
    let dir = tempfile::tempdir().expect("tmp");
    let (repo, tip) = history(dir.path(), 7);
    let one = dir.path().join("one");
    let a = Index::open_dir(one.clone()).expect("index");
    a.set_message_index(true);
    a.update(&repo, &tip, &|_| {}).expect("one file");
    assert_eq!(column(&one).len(), 1);

    let rolled = dir.path().join("rolled");
    super::writer::TEST_TOC.with(|t| t.set(Some(2)));
    let b = Index::open_dir(rolled.clone()).expect("index");
    b.set_message_index(true);
    let built = b.update(&repo, &tip, &|_| {});
    super::writer::TEST_TOC.with(|t| t.set(None));
    built.expect("rolled build");
    assert_eq!(column(&rolled).len(), 4, "seven blocks at two per file");
    let want = items(&one);
    assert_eq!(want.len(), 7);
    assert_eq!(items(&rolled), want);
}

/// A block the writer cannot write ends the text stream there: no more texts are read and
/// compressed into a column that already has a hole, and the update fails.
#[test]
fn a_failed_message_block_stops_the_text_stream() {
    struct Counting<'r>(&'r Repo, std::cell::Cell<usize>);
    impl crate::CommitSource for Counting<'_> {
        fn rev_list(&self, rev: &str) -> std::io::Result<Vec<String>> {
            crate::CommitSource::rev_list(self.0, rev)
        }
        fn record(&self, id: &str) -> std::io::Result<crate::CommitRecord> {
            crate::CommitSource::record(self.0, id)
        }
        fn stream_texts(
            &self,
            ids: &[String],
            emit: &mut dyn FnMut(gitterrace_git::CommitText) -> std::io::Result<()>,
        ) -> std::io::Result<bool> {
            crate::CommitSource::stream_texts(self.0, ids, &mut |t| {
                self.1.set(self.1.get() + 1);
                emit(t)
            })
        }
    }
    let dir = tempfile::tempdir().expect("tmp");
    let (repo, tip) = history(dir.path(), 7);
    let src = Counting(&repo, std::cell::Cell::new(0));
    let idx = Index::open_dir(dir.path().join("c")).expect("index");
    idx.set_message_index(true);
    super::writer::TEST_FAIL_AT.with(|f| f.set(Some(1)));
    let r = idx.update(&src, &tip, &|_| {});
    super::writer::TEST_FAIL_AT.with(|f| f.set(None));
    assert!(r.is_err(), "the update reports the failed block");
    assert_eq!(
        src.1.get(),
        2,
        "the stream stops at the push whose block failed"
    );
}
