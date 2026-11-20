//! An update halted mid-stream leaves the cache as it was.

use std::cell::Cell;
use std::io;

use crate::{CommitRecord, CommitSource, Index};

/// A source whose commits each touch their own path.
struct Paths {
    ids: Vec<String>,
    tag: &'static str,
}

impl Paths {
    fn new(tag: &'static str, n: usize) -> Self {
        Self { ids: (0..n).map(|i| format!("{tag}{i:039x}")).collect(), tag }
    }
}

impl CommitSource for Paths {
    fn rev_list(&self, _rev: &str) -> io::Result<Vec<String>> {
        Ok(self.ids.clone())
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        Ok(CommitRecord {
            id: id.to_owned(),
            files: vec![crate::FileStat {
                path: format!("{}/{id}.rs", self.tag).into(),
                additions: 1,
                ..crate::FileStat::default()
            }],
            ..CommitRecord::default()
        })
    }
}

/// A source that halts `idx` while it reads its `at`th record, as a stop arriving mid-batch.
struct HaltAt<'a> {
    inner: Paths,
    idx: &'a Index,
    at: usize,
    read: Cell<usize>,
}

impl CommitSource for HaltAt<'_> {
    fn rev_list(&self, rev: &str) -> io::Result<Vec<String>> {
        self.inner.rev_list(rev)
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        self.read.set(self.read.get() + 1);
        if self.read.get() == self.at {
            self.idx.halt();
        }
        self.inner.record(id)
    }
}

/// Every file in the cache but the lock, with its bytes.
fn snapshot(dir: &std::path::Path) -> io::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir)? {
        let name = e?.file_name().to_string_lossy().into_owned();
        if name != crate::store::LOCK_FILE {
            out.push((name.clone(), std::fs::read(dir.join(&name))?));
        }
    }
    out.sort();
    Ok(out)
}

/// Guard class: any write step of an update that runs after a halt, whether an append, a
/// fresh build, the message column or the tip marker. Both paths are stopped mid-stream and
/// must leave the cache byte for byte as it was, still readable.
#[test]
fn a_halted_update_leaves_the_previous_set_unchanged() -> io::Result<()> {
    // An extension (same history, more commits) and a rewrite (other history): append and build.
    for (tag, n) in [("a", 3 * crate::BATCH), ("b", 3 * crate::BATCH)] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("gitterrace");
        Index::open_dir(path.clone())?.update(&Paths::new("a", 10), "HEAD", &|_| {})?;
        let before = snapshot(&path)?;
        let idx = Index::open_dir(path.clone())?;
        let src = HaltAt {
            inner: Paths::new(tag, n),
            idx: &idx,
            at: crate::BATCH + 5,
            read: Cell::new(0),
        };
        let err = idx.update(&src, "HEAD", &|_| {}).expect_err("a halted update fails");
        assert_eq!(err.kind(), io::ErrorKind::Interrupted, "{tag}: {err}");
        assert!(src.read.get() < n, "{tag}: read every commit after the halt");
        let after = snapshot(&path)?;
        let names = |s: &[(String, Vec<u8>)]| s.iter().map(|f| f.0.clone()).collect::<Vec<_>>();
        assert_eq!(names(&after), names(&before), "{tag}: files changed");
        assert!(after == before, "{tag}: file contents changed");
        assert!(idx.update(&src, "HEAD", &|_| {}).is_err(), "{tag}: a halted handle updated again");
        let fresh = Index::open_dir(path)?;
        assert_eq!(
            fresh.pending(&Paths::new("a", 10), "HEAD")?,
            0,
            "{tag}: previous set unreadable"
        );
    }
    Ok(())
}
