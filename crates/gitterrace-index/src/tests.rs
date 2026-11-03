//! Unit tests for the index.

use std::cell::{Cell, RefCell};
use std::io;

use crate::{CommitRecord, CommitSource, Index, Progress};

struct Fake {
    ids: RefCell<Vec<String>>,
    built: Cell<usize>,
    batches: Cell<usize>,
}

impl Fake {
    fn new(ids: &[&str]) -> Self {
        Self {
            ids: RefCell::new(ids.iter().map(|s| (*s).to_owned()).collect()),
            built: Cell::new(0),
            batches: Cell::new(0),
        }
    }
}

impl CommitSource for Fake {
    fn rev_list(&self, _rev: &str) -> io::Result<Vec<String>> {
        Ok(self.ids.borrow().clone())
    }

    fn resolve(&self, _rev: &str) -> Option<String> {
        self.ids.borrow().last().cloned()
    }

    fn records(&self, ids: &[String]) -> io::Result<Vec<CommitRecord>> {
        self.batches.set(self.batches.get() + 1);
        ids.iter().map(|id| self.record(id)).collect()
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        self.built.set(self.built.get() + 1);
        Ok(CommitRecord {
            id: id.to_owned(),
            author: "a".into(),
            email: "a@x".into(),
            ..CommitRecord::default()
        })
    }
}

fn ids(rs: &[CommitRecord]) -> Vec<&str> {
    rs.iter().map(|r| r.id.as_str()).collect()
}

/// A source whose commits each touch their own paths, shareable across threads.
struct Paths {
    ids: Vec<String>,
    tag: &'static str,
    fail: std::sync::Mutex<std::collections::HashSet<String>>,
    shallow: std::sync::Mutex<Vec<String>>,
}

impl Paths {
    fn new(tag: &'static str, n: usize) -> Self {
        Self {
            ids: (0..n).map(|i| format!("{tag}{i:039x}")).collect(),
            tag,
            fail: std::sync::Mutex::default(),
            shallow: std::sync::Mutex::default(),
        }
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
                additions: u32::from(
                    !self
                        .shallow
                        .lock()
                        .map_err(|_| io::Error::other("poisoned"))?
                        .contains(&id.to_owned()),
                ),
                ..crate::FileStat::default()
            }],
            ..CommitRecord::default()
        })
    }

    fn records(&self, ids: &[String]) -> io::Result<Vec<CommitRecord>> {
        let fail = self
            .fail
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?
            .clone();
        ids.iter()
            .filter(|id| !fail.contains(*id))
            .map(|id| self.record(id))
            .collect()
    }

    fn shallow_ids(&self) -> Vec<String> {
        self.shallow.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// Two processes indexing one repository (two `Index` handles on one folder, as `serve`
/// beside `mcp`) never hand one path id to two paths.
#[test]
fn two_writers_keep_every_path_with_its_record() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let (a, b) = (Paths::new("a", 3000), Paths::new("b", 3000));
    std::thread::scope(|s| {
        let one = s.spawn(|| Index::open_dir(path.clone())?.update(&a, "HEAD", &|_| {}));
        let two = s.spawn(|| Index::open_dir(path.clone())?.update(&b, "HEAD", &|_| {}));
        for h in [one, two] {
            h.join()
                .map_err(|_| io::Error::other("writer panicked"))??;
        }
        Ok::<(), io::Error>(())
    })?;
    let idx = Index::open_dir(path)?;
    for src in [&a, &b] {
        let got = idx.records(src, "HEAD")?;
        assert_eq!(got.len(), 3000);
        for r in got {
            assert_eq!(&*r.files[0].path, format!("{}/{}.rs", src.tag, r.id));
        }
    }
    Ok(())
}

/// While another writer holds the lock, an update waits instead of appending.
#[test]
fn update_waits_for_the_writer_lock() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let held = crate::store::WriteLock::take(idx.dir())?;
    let src = Paths::new("a", 1);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        let writer = s.spawn(|| tx.send(idx.update(&src, "HEAD", &|_| {})));
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "update ran under a held lock"
        );
        drop(held);
        let done = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .map_err(io::Error::other)?;
        writer
            .join()
            .map_err(|_| io::Error::other("writer panicked"))?
            .map_err(io::Error::other)?;
        done
    })?;
    assert_eq!(idx.records(&src, "HEAD")?.len(), 1);
    Ok(())
}

/// A format reset deletes the record files: it must wait for a writer that holds the lock.
#[test]
fn format_reset_waits_for_the_writer_lock() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let idx = Index::open_dir(path.clone())?;
    idx.update(&Paths::new("a", 1), "HEAD", &|_| {})?;
    std::fs::write(path.join(crate::store::VERSION_FILE), "0\n")?;
    let held = crate::store::WriteLock::take(&path)?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(|| tx.send(Index::open_dir(path.clone()).map(drop)));
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "the reset ran under a held lock"
        );
        assert!(path.join(crate::store::COMMITS_FILE).exists());
        drop(held);
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .map_err(io::Error::other)?
    })
}

/// A build killed at any byte of either file (a torn record, a torn path, paths written
/// but their records not) resumes to files byte-identical to an uninterrupted build. The
/// bench harness kills servers mid-index, so a resume that renumbered a path or reordered
/// a row would show as an index that differs from the reference.
#[test]
fn a_build_killed_at_any_byte_resumes_byte_identical() -> io::Result<()> {
    let read = |d: &std::path::Path| -> io::Result<(Vec<u8>, Vec<u8>)> {
        Ok((
            std::fs::read(d.join(crate::store::COMMITS_FILE))?,
            std::fs::read(d.join(crate::store::PATHS_FILE))?,
        ))
    };
    let src = Paths::new("a", 600);
    let whole = tempfile::tempdir()?;
    let idx = Index::open_dir(whole.path().join("gitterrace"))?;
    idx.update(&src, "HEAD", &|_| {})?;
    let (commits, paths) = read(idx.dir())?;
    let cuts = |len: usize| [0, 1, len / 3, len / 2 + 7, len - 1, len];
    for c in cuts(commits.len()) {
        for p in cuts(paths.len()) {
            let dir = tempfile::tempdir()?;
            let path = dir.path().join("gitterrace");
            Index::open_dir(path.clone())?;
            std::fs::write(path.join(crate::store::COMMITS_FILE), &commits[..c])?;
            std::fs::write(path.join(crate::store::PATHS_FILE), &paths[..p])?;
            // Commit i names path i, and paths are written before their records: fewer
            // whole path lines than record lines is not a state a kill can leave.
            let lines = |b: &[u8]| b.split(|&x| x == b'\n').count();
            if lines(&commits[..c]) > lines(&paths[..p]) {
                continue;
            }
            let resumed = Index::open_dir(path)?;
            resumed.update(&src, "HEAD", &|_| {})?;
            let got = read(resumed.dir())?;
            assert!(
                got.0 == commits && got.1 == paths,
                "resume after a cut at commits {c} / paths {p} differs"
            );
        }
    }
    Ok(())
}

/// A commit the source cannot read is not stored, stays pending and is built next time.
#[test]
fn an_unreadable_commit_stays_pending_and_is_retried() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Paths::new("a", 3);
    let bad = src.ids[1].clone();
    src.fail
        .lock()
        .map_err(|_| io::Error::other("poisoned"))?
        .insert(bad.clone());
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(idx.pending(&src, "HEAD")?, 1);
    assert_eq!(idx.records(&src, "HEAD")?.len(), 2);
    src.fail
        .lock()
        .map_err(|_| io::Error::other("poisoned"))?
        .clear();
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(idx.pending(&src, "HEAD")?, 0);
    assert!(idx.records(&src, "HEAD")?.iter().any(|r| r.id == bad));
    Ok(())
}

/// A row built at a shallow boundary is rebuilt once the clone has the parents.
#[test]
fn a_shallow_boundary_row_is_rebuilt_after_unshallow() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Paths::new("a", 2);
    let edge = src.ids[0].clone();
    src.shallow
        .lock()
        .map_err(|_| io::Error::other("poisoned"))?
        .push(edge.clone());
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(
        idx.pending(&src, "HEAD")?,
        0,
        "a boundary row is done while shallow"
    );
    assert_eq!(idx.records(&src, "HEAD")?[0].files[0].additions, 0);
    src.shallow
        .lock()
        .map_err(|_| io::Error::other("poisoned"))?
        .clear();
    assert_eq!(idx.pending(&src, "HEAD")?, 1);
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(idx.pending(&src, "HEAD")?, 0);
    let got = idx.records(&src, "HEAD")?;
    assert_eq!(got.len(), 2);
    assert_eq!(
        (got[0].id.as_str(), got[0].files[0].additions),
        (edge.as_str(), 1)
    );
    Ok(())
}

/// Records dated outside what a calendar can chart are pinned to the edges on read.
#[test]
fn absurd_record_times_are_pinned_on_read() {
    let latest = 1_800_000_000;
    assert_eq!(crate::plausible_time(i64::MAX, latest), latest);
    assert_eq!(crate::plausible_time(i64::MIN, latest), 0);
    assert_eq!(crate::plausible_time(1_700_000_000, latest), 1_700_000_000);
}

/// The folder holds exactly the files `CACHE_FILES` documents.
#[test]
fn cache_folder_holds_the_documented_files() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    idx.update(&Fake::new(&["a"]), "HEAD", &|_| {})?;
    let mut lines = crate::LineCache::load(idx.dir());
    lines.map.insert("abc".into(), 3);
    lines.dirty = true;
    lines.save(idx.dir())?;
    let mut have: Vec<String> = std::fs::read_dir(idx.dir())?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<_>>()?;
    have.sort();
    let mut want: Vec<&str> = crate::CACHE_FILES.iter().map(|(n, _)| *n).collect();
    want.sort_unstable();
    assert_eq!(have, want);
    Ok(())
}

#[test]
fn pending_counts_uncached_commits_of_a_rev() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().to_path_buf())?;
    let src = Fake::new(&["a", "b"]);
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(idx.pending(&src, "HEAD")?, 0);
    // Another branch shares a and b and adds c.
    src.ids.borrow_mut().push("c".into());
    assert_eq!(idx.pending(&src, "side")?, 1);
    idx.update(&src, "side", &|_| {})?;
    assert_eq!(idx.pending(&src, "side")?, 0);
    assert_eq!(ids(&idx.records(&src, "side")?), ["a", "b", "c"]);
    Ok(())
}

#[test]
fn update_is_incremental_and_ordered() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Fake::new(&["a", "b"]);
    idx.update(&src, "HEAD", &|_| {})?;
    src.ids.borrow_mut().push("c".into());
    let last = Cell::new(Progress::default());
    idx.update(&src, "HEAD", &|p| last.set(p))?;
    assert_eq!(src.built.get(), 3);
    assert_eq!(last.get(), Progress { done: 1, total: 1 });
    assert_eq!(ids(&idx.records(&src, "HEAD")?), ["a", "b", "c"]);
    Ok(())
}

#[test]
fn stored_ids_match_parsed_ids_and_parallel_parse_keeps_order() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let names: Vec<String> = (0..4000).map(|i| format!("{i:040x}")).collect();
    let wanted: Vec<&str> = names.iter().map(String::as_str).collect();
    let src = Fake::new(&wanted);
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(crate::store::stored_ids(idx.dir())?.ids.len(), 4000);
    let (parsed, _) = crate::store::read_records(idx.dir(), 0)?;
    assert_eq!(ids(&parsed), wanted);
    Ok(())
}

/// The per-batch setup cost is fixed per worker, so the number of batches must stay small:
/// building N commits takes about N / BATCH calls, not one per few commits.
#[test]
fn update_builds_in_few_large_batches() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let names: Vec<String> = (0..5000).map(|i| format!("{i:040x}")).collect();
    let wanted: Vec<&str> = names.iter().map(String::as_str).collect();
    let src = Fake::new(&wanted);
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(src.batches.get(), 5000_usize.div_ceil(crate::BATCH));
    assert!(src.batches.get() <= 3);
    Ok(())
}

/// A path is stored once however many records name it, and survives the round trip with
/// renames, across a reopened cache.
#[test]
fn paths_are_interned_and_round_trip() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Fake::new(&["a", "b", "c"]);
    let stat = |p: &str, old: Option<&str>| crate::FileStat {
        path: p.into(),
        old: old.map(Into::into),
        additions: 2,
        ..crate::FileStat::default()
    };
    let mut out = crate::store::Appender::open(idx.dir(), 0)?;
    for (id, files) in [
        ("a", vec![stat("src/x.rs", None)]),
        ("b", vec![stat("src/x.rs", None), stat("y.rs", None)]),
        ("c", vec![stat("z.rs", Some("src/x.rs"))]),
    ] {
        let mut r = src.record(id)?;
        r.files = files;
        out.push(&r, false)?;
    }
    out.flush()?;
    let paths = std::fs::read_to_string(idx.dir().join("paths.jsonl"))?;
    assert_eq!(paths.lines().count(), 3, "{paths}");
    let got = idx.records(&src, "HEAD")?;
    assert_eq!(&*got[1].files[1].path, "y.rs");
    assert_eq!(got[2].files[0].old.as_deref(), Some("src/x.rs"));
    assert_eq!(got[0].files[0].additions, 2);
    Ok(())
}

/// A build never touches a cache a newer build wrote, and says why. The same holds when the
/// newer build takes the cache over while this one is already running.
#[test]
fn a_newer_cache_is_left_byte_identical() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let idx = Index::open_dir(path.clone())?;
    let src = Fake::new(&["a", "b"]);
    idx.update(&src, "HEAD", &|_| {})?;
    let snapshot = |p: &std::path::Path| -> io::Result<Vec<(String, Vec<u8>)>> {
        let mut v = Vec::new();
        for e in std::fs::read_dir(p)? {
            let e = e?;
            v.push((
                e.file_name().to_string_lossy().into_owned(),
                std::fs::read(e.path())?,
            ));
        }
        v.sort();
        Ok(v)
    };
    // A newer build takes over while `idx` is open.
    std::fs::write(path.join("VERSION"), "99\n")?;
    src.ids.borrow_mut().push("c".into());
    let before = snapshot(&path)?;
    let err = idx.update(&src, "HEAD", &|_| {}).expect_err("must refuse");
    assert!(err.to_string().contains("newer"), "{err}");
    assert!(idx.records(&src, "HEAD").is_err());
    assert!(idx.pending(&src, "HEAD").is_err());
    // A build that starts after it sees the same.
    let late = Index::open_dir(path.clone())?;
    assert_eq!(late.newer_format(), Some("99"));
    assert!(late.update(&src, "HEAD", &|_| {}).is_err());
    assert_eq!(snapshot(&path)?, before);
    Ok(())
}

#[test]
fn unreadable_rows_are_counted_not_silent() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let idx = Index::open_dir(path.clone())?;
    let src = Fake::new(&["a", "b"]);
    idx.update(&src, "HEAD", &|_| {})?;
    let file = path.join("commits.jsonl");
    let mut text = std::fs::read_to_string(&file)?;
    text.push_str("{\"id\":\"z\",\"files\":[{\"path\":\"old/style.rs\"}]}\n");
    std::fs::write(&file, text)?;
    // The bad row is dropped, the good ones survive; the count goes to stderr.
    assert_eq!(ids(&idx.records(&src, "HEAD")?), ["a", "b"]);
    Ok(())
}

#[test]
fn generation_moves_when_records_are_added() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Fake::new(&["a"]);
    idx.update(&src, "HEAD", &|_| {})?;
    let before = idx.generation();
    src.ids.borrow_mut().push("b".into());
    idx.update(&src, "HEAD", &|_| {})?;
    assert!(idx.generation() > before);
    Ok(())
}

#[test]
fn unreachable_records_are_ignored() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Fake::new(&["a", "b", "c"]);
    idx.update(&src, "HEAD", &|_| {})?;
    src.ids.replace(vec!["a".into(), "c".into()]);
    assert_eq!(ids(&idx.records(&src, "HEAD")?), ["a", "c"]);
    Ok(())
}

#[test]
fn torn_last_line_is_dropped_and_rebuilt() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let idx = Index::open_dir(path.clone())?;
    let src = Fake::new(&["a", "b"]);
    idx.update(&src, "HEAD", &|_| {})?;
    let file = path.join("commits.jsonl");
    let mut text = std::fs::read_to_string(&file)?;
    text.pop();
    text.truncate(text.len() - 5);
    std::fs::write(&file, text)?;
    assert_eq!(ids(&idx.records(&src, "HEAD")?), ["a"]);
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(ids(&idx.records(&src, "HEAD")?), ["a", "b"]);
    assert_eq!(std::fs::read_to_string(&file)?.lines().count(), 2);
    Ok(())
}

#[test]
fn version_mismatch_rebuilds() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let src = Fake::new(&["a"]);
    Index::open_dir(path.clone())?.update(&src, "HEAD", &|_| {})?;
    std::fs::write(path.join("VERSION"), "0\n")?;
    let idx = Index::open_dir(path.clone())?;
    assert!(idx.records(&src, "HEAD")?.is_empty());
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(src.built.get(), 2);
    assert_eq!(
        std::fs::read_to_string(path.join("VERSION"))?.trim(),
        crate::store::FORMAT_VERSION
    );
    Ok(())
}

/// Plants a link at `name` inside a cache (or at the cache itself for `""`), runs the
/// whole cache cycle and asserts the link target outside is untouched.
#[cfg(unix)]
fn planted(name: &str) -> io::Result<()> {
    const PRECIOUS: &str = "precious user data\nno newline tail";
    let repo = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let victim = outside.path().join("victim");
    std::fs::write(&victim, PRECIOUS)?;
    let dir = repo.path().join("gitterrace");
    if name.is_empty() {
        std::os::unix::fs::symlink(outside.path(), &dir)?;
    } else {
        std::fs::create_dir(&dir)?;
        if name != "VERSION" {
            std::fs::write(dir.join("VERSION"), "0\n")?;
        }
        std::os::unix::fs::symlink(&victim, dir.join(name))?;
    }
    let src = Fake::new(&["a"]);
    if let Ok(idx) = Index::open_dir(dir.clone())
        && idx.update(&src, "HEAD", &|_| {}).is_ok()
    {
        let mut lines = crate::LineCache::load(idx.dir());
        lines.map.insert("abc".into(), 3);
        lines.dirty = true;
        drop(lines.save(idx.dir()));
    }
    assert_eq!(std::fs::read_to_string(&victim)?, PRECIOUS, "{name}");
    assert_eq!(std::fs::read_dir(outside.path())?.count(), 1, "{name}");
    assert_eq!(src.built.get(), 1, "the cache still works for {name:?}");
    assert!(!dir.is_symlink(), "{name}");
    Ok(())
}

#[cfg(unix)]
#[test]
fn planted_links_never_write_outside() -> io::Result<()> {
    for name in [
        "",
        "VERSION",
        "commits.jsonl",
        "lines.json",
        "lines.json.tmp",
    ] {
        planted(name).map_err(|e| io::Error::other(format!("{name:?}: {e}")))?;
    }
    Ok(())
}
