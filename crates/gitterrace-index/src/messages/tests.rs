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
