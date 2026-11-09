//! The stored chain against the walk it replaces, and against every way a set changes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::feed::{ChainStep, DiskSource, InsightsSource};
use crate::seg::format::Kind;
use crate::seg::manifest::Manifest;
use crate::seg::read::Store;
use crate::{CommitRecord, CommitSource, FileStat, Index};

/// A history whose tip and shallow set the test moves. `order` is oldest first.
#[derive(Default)]
struct Hist {
    recs: RefCell<HashMap<String, CommitRecord>>,
    order: RefCell<Vec<String>>,
    shallow: RefCell<Vec<String>>,
}

fn id(n: usize) -> String {
    format!("{n:040x}")
}

fn file(path: &str, add: u32, del: u32) -> FileStat {
    FileStat {
        path: path.into(),
        additions: add,
        deletions: del,
        ..FileStat::default()
    }
}

impl Hist {
    /// Adds commit `n` on top of `parents` and makes it the tip's history end.
    fn commit(&self, n: usize, parents: &[usize], files: Vec<FileStat>) {
        let r = CommitRecord {
            id: id(n),
            author: "a".into(),
            email: "a@x".into(),
            time: 1_700_000_000 + i64::try_from(n).unwrap_or(0) * 60,
            merge: parents.len() > 1,
            parents: parents.iter().map(|p| id(*p)).collect(),
            files,
            ..CommitRecord::default()
        };
        self.recs.borrow_mut().insert(r.id.clone(), r);
        self.order.borrow_mut().push(id(n));
    }

    /// Keeps only the commits reachable from `tip` in the order (a reset).
    fn reset(&self, tip: usize) {
        let recs = self.recs.borrow();
        let mut keep = std::collections::HashSet::new();
        let mut stack = vec![id(tip)];
        while let Some(c) = stack.pop() {
            if keep.insert(c.clone()) {
                stack.extend(recs[&c].parents.iter().cloned());
            }
        }
        self.order.borrow_mut().retain(|c| keep.contains(c));
        let mut o = self.order.borrow_mut();
        let at = o.iter().position(|c| *c == id(tip)).unwrap_or(0);
        let t = o.remove(at);
        o.push(t);
    }
}

impl CommitSource for Hist {
    fn rev_list(&self, _rev: &str) -> io::Result<Vec<String>> {
        Ok(self.order.borrow().clone())
    }

    fn resolve(&self, _rev: &str) -> Option<String> {
        self.order.borrow().last().cloned()
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        let mut r = self.recs.borrow()[id].clone();
        // A boundary commit is a root while the clone is shallow.
        if self.shallow.borrow().iter().any(|s| s == id) {
            r.parents.clear();
        }
        Ok(r)
    }

    fn shallow_ids(&self) -> Vec<String> {
        self.shallow.borrow().clone()
    }
}

/// A linear history with a side branch merged in, a rename across languages (`a.c` to
/// `a.rs`), a binary file and a deleted path.
fn history() -> Hist {
    let h = Hist::default();
    h.commit(0, &[], vec![file("a.c", 10, 0), file("logo.png", 0, 0)]);
    h.commit(1, &[0], vec![file("b.py", 5, 0)]);
    let mut renamed = file("a.rs", 10, 0);
    renamed.old = Some("a.c".into());
    h.commit(2, &[1], vec![file("a.c", 0, 10), renamed]);
    h.commit(3, &[1], vec![file("side.go", 7, 0)]);
    h.commit(4, &[2, 3], vec![file("side.go", 7, 0)]);
    let mut gone = file("b.py", 0, 5);
    gone.deleted = true;
    h.commit(5, &[4], vec![gone, file("a.rs", 3, 1)]);
    h
}

fn disk(dir: &Path) -> DiskSource {
    let store = Store::open(dir).expect("open").expect("a set");
    DiskSource::open(store, &mut |p| Ok(p.to_vec())).expect("disk")
}

/// The stored steps as (oid, time, [(path, add, del)]), `None` without a current chain.
type Steps = Vec<(String, i64, Vec<(String, u32, u32)>)>;

fn stored(dir: &Path) -> Option<Steps> {
    let d = disk(dir);
    let mut out = Vec::new();
    let path = |p| d.path(p).map(|s| s.to_string()).unwrap_or_default();
    let had = d.chain(&mut |s: &ChainStep| {
        let files = s
            .files
            .iter()
            .map(|f| (path(f.path), f.add, f.del))
            .collect();
        out.push((crate::messages::hex(&s.oid), s.time, files));
    });
    assert!(d.failure().is_none(), "the chain read fails");
    had.then_some(out)
}

/// The same steps by walking first parents through the records, as the page did before.
fn walked(dir: &Path) -> Steps {
    let d = disk(dir);
    let path = |p| d.path(p).map(|s| s.to_string()).unwrap_or_default();
    let mut out = Vec::new();
    let mut at = d.len().checked_sub(1).map(|i| crate::feed::Rank(i as u32));
    while let Some(r) = at {
        let (_, files) = d.rows_for(vec![r]).next().expect("rows");
        let files = files.iter().map(|f| (path(f.path), f.add, f.del)).collect();
        let time = d.commit(r).expect("commit").time;
        out.push((d.commit_id(r).expect("id").to_string(), time, files));
        at = d.parent_ids(r).first().and_then(|p| d.commit_of(p));
    }
    out
}

fn build(h: &Hist) -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    Index::open_dir(dir.clone())
        .expect("index")
        .update(h, "HEAD", &|_| {})
        .expect("update");
    (tmp, dir)
}

fn chain_files(dir: &Path) -> usize {
    let m = Manifest::load(dir).expect("manifest").expect("set");
    m.live
        .iter()
        .filter(|f| f.kind == Kind::Chain as u8)
        .count()
}

/// A build stores the first-parent chain with the rows the walk reads, step for step: the
/// side commit is left out, the rename keeps both names, the binary and the deletion stay.
#[test]
fn a_build_stores_the_chain_the_walk_reads() {
    let h = history();
    let (_t, dir) = build(&h);
    assert_eq!(chain_files(&dir), 1);
    let got = stored(&dir).expect("a current chain");
    assert_eq!(got, walked(&dir));
    let oids: Vec<&str> = got.iter().map(|s| &s.0[36..]).collect();
    assert_eq!(oids, ["0005", "0004", "0002", "0001", "0000"]);
    assert!(got[2].2.contains(&("a.c".into(), 0, 10)));
    assert!(got[4].2.contains(&("logo.png".into(), 0, 0)));
}

/// An update that appends, then a reset to an older commit plus a new commit (a full
/// rebuild, since the old tip left the history): each time the chain equals a fresh build's.
#[test]
fn appends_and_a_reset_leave_the_chain_a_fresh_build_writes() {
    let h = history();
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    let idx = Index::open_dir(dir.clone()).expect("index");
    idx.update(&h, "HEAD", &|_| {}).expect("first");
    h.commit(6, &[5], vec![file("c.ts", 4, 0)]);
    idx.update(&h, "HEAD", &|_| {}).expect("append");
    assert_eq!(chain_files(&dir), 1, "the old chain is retired");
    assert_eq!(stored(&dir), Some(walked(&dir)));
    assert_eq!(stored(&dir), stored(&build(&h).1));
    h.reset(2);
    h.commit(7, &[2], vec![file("a.rs", 0, 2)]);
    idx.update(&h, "HEAD", &|_| {}).expect("reset");
    let fresh = build(&h);
    assert_eq!(stored(&dir), Some(walked(&dir)));
    assert_eq!(stored(&dir), stored(&fresh.1));
    assert_eq!(stored(&dir).map(|s| s.len()), Some(4));
}

/// A rewritten history and a deepened shallow clone both leave a chain for the new set only:
/// the old one is retired and the new one matches the walk.
#[test]
fn a_rewrite_and_a_deepened_clone_replace_the_chain() {
    let h = history();
    h.shallow.borrow_mut().extend([id(2), id(3)]);
    *h.order.borrow_mut() = [2, 3, 4, 5].map(id).to_vec();
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    let idx = Index::open_dir(dir.clone()).expect("index");
    idx.update(&h, "HEAD", &|_| {}).expect("shallow");
    assert_eq!(stored(&dir).map(|s| s.len()), Some(3));
    h.shallow.borrow_mut().clear();
    *h.order.borrow_mut() = (0..6).map(id).collect();
    idx.update(&h, "HEAD", &|_| {}).expect("deepened");
    assert_eq!(chain_files(&dir), 1);
    assert_eq!(stored(&dir), Some(walked(&dir)));
    assert_eq!(stored(&dir).map(|s| s.len()), Some(5));
    // Rewritten: commit 5 replaced by another commit on 4.
    h.reset(4);
    h.commit(8, &[4], vec![file("z.md", 1, 0)]);
    idx.update(&h, "HEAD", &|_| {}).expect("rewrite");
    assert_eq!(chain_files(&dir), 1);
    assert_eq!(stored(&dir), Some(walked(&dir)));
    assert_eq!(stored(&dir), stored(&build(&h).1));
}

/// A chain whose meta names another set is stale: the source has no chain then, and the
/// next update writes a current one.
#[test]
fn a_chain_written_for_another_set_is_not_served() {
    let h = history();
    let (_t, dir) = build(&h);
    let mut m = Manifest::load(&dir).expect("manifest").expect("set");
    m.fingerprint ^= 1;
    m.commit(&dir).expect("commit");
    assert_eq!(stored(&dir), None);
    let idx = Index::open_dir(dir.clone()).expect("index");
    crate::chain::sync(idx.dir()).expect("sync");
    assert_eq!(stored(&dir), Some(walked(&dir)));
}

/// A cache of format 8 (no chain file kind) is deleted whole on open and rebuilt with one.
#[test]
fn a_format_8_cache_is_rebuilt_with_a_chain() {
    let h = history();
    let (_t, dir) = build(&h);
    std::fs::write(dir.join("VERSION"), "8\n").expect("old version");
    let idx = Index::open_dir(dir.clone()).expect("reopen");
    assert!(Manifest::load(&dir).expect("load").is_none(), "wiped");
    idx.update(&h, "HEAD", &|_| {}).expect("rebuild");
    assert_eq!(stored(&dir), Some(walked(&dir)));
}

/// Hostile step bytes behind a valid crc: a rank that does not fall, a path id past the
/// strings and a cut step are each refused, never served.
#[test]
fn hostile_chain_steps_are_unreadable() {
    let hist = history();
    let (_t, dir) = build(&hist);
    let store = Store::open(&dir).expect("open").expect("set");
    let chain = store.chain.as_ref().expect("chain");
    let entry = chain.blocks(crate::seg::format::section::CHAIN)[0];
    let mut buf = Vec::new();
    let good = chain.read_block(&entry, &mut buf).expect("block").to_vec();
    let check = |bytes: &[u8], items: u32, paths: usize| {
        let mut cur = crate::seg::bounds::Cursor::new(bytes, "t", 0);
        let mut rank_at = 6;
        let mut res = Ok(());
        for _ in 0..items {
            res = super::step(&mut cur, 6, &mut rank_at, paths, i64::MAX, &mut |_| {});
            if res.is_err() {
                break;
            }
        }
        res
    };
    assert!(check(&good, entry.items, 64).is_ok());
    let mut rank = good.clone();
    rank[..4].copy_from_slice(&9u32.to_le_bytes());
    assert!(check(&rank, entry.items, 64).is_err_and(|err| err.is_unreadable()));
    assert!(check(&good, entry.items, 1).is_err_and(|err| err.is_unreadable()));
    assert!(check(&good[..good.len() - 1], entry.items, 64).is_err());
}
