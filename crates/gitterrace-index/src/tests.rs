//! Unit tests for the index.

use std::cell::{Cell, RefCell};
use std::io;

use crate::{CommitRecord, CommitSource, Index, Progress};

struct Fake {
    ids: RefCell<Vec<String>>,
    built: Cell<usize>,
    batches: Cell<usize>,
}

/// A 40-digit commit id for a short test name, so the set can store it.
fn oid(name: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    if name.len() == 40 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
        return name.to_owned();
    }
    let hex: String = name
        .bytes()
        .flat_map(|b| [HEX[usize::from(b >> 4)], HEX[usize::from(b & 15)]])
        .map(char::from)
        .collect();
    format!("{hex:0>40}")
}

impl Fake {
    fn new(ids: &[&str]) -> Self {
        Self {
            ids: RefCell::new(ids.iter().map(|s| oid(s)).collect()),
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

fn ids(rs: &[CommitRecord]) -> Vec<String> {
    rs.iter().map(|r| r.id.clone()).collect()
}

fn oids(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| oid(n)).collect()
}

/// A source whose commits each touch their own paths, shareable across threads.
pub(crate) struct Paths {
    ids: Vec<String>,
    tag: &'static str,
    fail: std::sync::Mutex<std::collections::HashSet<String>>,
    shallow: std::sync::Mutex<Vec<String>>,
}

impl Paths {
    pub(crate) fn new(tag: &'static str, n: usize) -> Self {
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
        let fail = self.fail.lock().map_err(|_| io::Error::other("poisoned"))?.clone();
        ids.iter().filter(|id| !fail.contains(*id)).map(|id| self.record(id)).collect()
    }

    fn shallow_ids(&self) -> Vec<String> {
        self.shallow.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// Two processes indexing one repository (two `Index` handles on one folder, as `serve`
/// beside `mcp`) take turns: the set ends up holding one history whole, every path with its
/// own commit, never a mix of both writers' ids.
#[test]
fn two_writers_leave_one_whole_history() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let (a, b) = (Paths::new("a", 3000), Paths::new("b", 3000));
    std::thread::scope(|s| {
        let one = s.spawn(|| Index::open_dir(path.clone())?.update(&a, "HEAD", &|_| {}));
        let two = s.spawn(|| Index::open_dir(path.clone())?.update(&b, "HEAD", &|_| {}));
        for h in [one, two] {
            h.join().map_err(|_| io::Error::other("writer panicked"))??;
        }
        Ok::<(), io::Error>(())
    })?;
    let idx = Index::open_dir(path)?;
    let whole: Vec<&Paths> = [&a, &b]
        .into_iter()
        .filter(|src| idx.pending(*src, "HEAD").is_ok_and(|n| n == 0))
        .collect();
    assert_eq!(whole.len(), 1, "exactly one history is held whole");
    let got = idx.records(whole[0], "HEAD")?;
    assert_eq!(got.len(), 3000);
    for r in got {
        assert_eq!(&*r.files[0].path, format!("{}/{}.rs", whole[0].tag, r.id));
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
            rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(),
            "update ran under a held lock"
        );
        drop(held);
        let done = rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(io::Error::other)?;
        writer
            .join()
            .map_err(|_| io::Error::other("writer panicked"))?
            .map_err(io::Error::other)?;
        done
    })?;
    assert_eq!(idx.records(&src, "HEAD")?.len(), 1);
    Ok(())
}

/// A format reset deletes the segment set: it must wait for a writer that holds the lock.
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
            rx.recv_timeout(std::time::Duration::from_millis(300)).is_err(),
            "the reset ran under a held lock"
        );
        assert!(path.join(crate::seg::manifest::MANIFEST).exists());
        drop(held);
        rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(io::Error::other)?
    })
}

/// A commit the source cannot read is not stored, stays pending and is built next time.
#[test]
fn an_unreadable_commit_stays_pending_and_is_retried() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Paths::new("a", 3);
    let bad = src.ids[1].clone();
    src.fail.lock().map_err(|_| io::Error::other("poisoned"))?.insert(bad.clone());
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(idx.pending(&src, "HEAD")?, 1);
    assert_eq!(idx.records(&src, "HEAD")?.len(), 2);
    src.fail.lock().map_err(|_| io::Error::other("poisoned"))?.clear();
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
    src.shallow.lock().map_err(|_| io::Error::other("poisoned"))?.push(edge.clone());
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(idx.pending(&src, "HEAD")?, 0, "a boundary row is done while shallow");
    assert_eq!(idx.records(&src, "HEAD")?[0].files[0].additions, 0);
    src.shallow.lock().map_err(|_| io::Error::other("poisoned"))?.clear();
    // A moved boundary changes topo ranks, so the whole set is rebuilt.
    assert_eq!(idx.pending(&src, "HEAD")?, 2);
    idx.update(&src, "HEAD", &|_| {})?;
    assert_eq!(idx.pending(&src, "HEAD")?, 0);
    let got = idx.records(&src, "HEAD")?;
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].id.as_str(), got[0].files[0].additions), (edge.as_str(), 1));
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
    crate::store::claim(idx.dir(), std::path::Path::new("/repos/a/.git"))?;
    idx.save_state(crate::StateFile::Chain, 1, b"x")?;
    idx.checkpoint_add(&[], "abc", false, "", 1 << 20, 1)?;
    idx.viewed_set(&[("a".to_owned(), "h".to_owned())], true, 1)?;
    // The review basket is written by a later build: its file is named here, not made.
    std::fs::write(idx.dir().join("findings"), "[]")?;
    let mut have: Vec<String> = std::fs::read_dir(idx.dir())?
        .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<_>>()?;
    have.sort();
    // The documented files plus the manifest and the files it names.
    let mut want: Vec<String> = crate::CACHE_FILES.iter().map(|(n, _)| (*n).to_owned()).collect();
    want.push(crate::seg::manifest::MANIFEST.to_owned());
    let m = crate::seg::manifest::Manifest::load(idx.dir())
        .map_err(io::Error::other)?
        .ok_or_else(|| io::Error::other("no manifest"))?;
    want.extend(m.live.into_iter().map(|f| f.name));
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
    src.ids.borrow_mut().push(oid("c"));
    assert_eq!(idx.pending(&src, "side")?, 1);
    idx.update(&src, "side", &|_| {})?;
    assert_eq!(idx.pending(&src, "side")?, 0);
    assert_eq!(ids(&idx.records(&src, "side")?), oids(&["a", "b", "c"]));
    Ok(())
}

#[test]
fn update_is_incremental_and_ordered() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Fake::new(&["a", "b"]);
    idx.update(&src, "HEAD", &|_| {})?;
    src.ids.borrow_mut().push(oid("c"));
    let last = Cell::new(Progress::default());
    idx.update(&src, "HEAD", &|p| last.set(p))?;
    assert_eq!(src.built.get(), 3);
    assert_eq!(last.get(), Progress { done: 1, total: 1 });
    assert_eq!(ids(&idx.records(&src, "HEAD")?), oids(&["a", "b", "c"]));
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
            v.push((e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path())?));
        }
        v.sort();
        Ok(v)
    };
    // A newer build takes over while `idx` is open.
    std::fs::write(path.join("VERSION"), "v99\n")?;
    src.ids.borrow_mut().push(oid("c"));
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
fn generation_moves_when_records_are_added() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    let src = Fake::new(&["a"]);
    idx.update(&src, "HEAD", &|_| {})?;
    let before = idx.generation();
    src.ids.borrow_mut().push(oid("b"));
    idx.update(&src, "HEAD", &|_| {})?;
    assert!(idx.generation() > before);
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
        crate::store::version_stamp()
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
    for name in ["", "VERSION", "MANIFEST", "lines.json", "lines.json.tmp"] {
        planted(name).map_err(|e| io::Error::other(format!("{name:?}: {e}")))?;
    }
    Ok(())
}

/// Every step of a fresh build is timed and named, so a bench can say which one is slow.
#[test]
fn a_fresh_build_times_every_step_by_name() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let idx = Index::open_dir(dir.path().join("gitterrace"))?;
    idx.update(&Fake::new(&["a", "b"]), "HEAD", &|_| {})?;
    let (steps, running) = idx.phases();
    let names: Vec<_> = steps.iter().map(|p| p.name).collect();
    let want = ["rev_list", "held", "stream", "stream.fold", "finish", "messages", "tip"];
    assert_eq!(names, want);
    assert_eq!(running, None);
    assert!(steps.iter().all(|p| p.secs >= 0.0));
    Ok(())
}

/// An inline segment merge, as the index child runs it, finishes inside the update: twelve
/// appends push the set past the merge limit, and each update returns. Red when the merge runs
/// under the update's own writer lock, which it then waits for forever.
#[test]
fn an_inline_merge_past_the_segment_limit_finishes_the_update() -> io::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("gitterrace");
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let run = || -> io::Result<usize> {
            let idx = Index::open_dir(path)?;
            idx.set_merge_inline(true);
            let names: Vec<String> = (0..12).map(|i| format!("c{i}")).collect();
            for n in 1..=names.len() {
                let these: Vec<&str> = names[..n].iter().map(String::as_str).collect();
                idx.update(&Fake::new(&these), "HEAD", &|_| {})?;
            }
            crate::seg::manifest::Manifest::load(idx.dir()).map_err(io::Error::other)?.map_or(
                Ok(0),
                |m| {
                    let seg = crate::seg::format::Kind::Segment as u8;
                    Ok(m.live.iter().filter(|f| f.kind == seg).count())
                },
            )
        };
        drop(tx.send(run()));
    });
    let segs = rx.recv_timeout(std::time::Duration::from_secs(30)).map_err(|_| {
        io::Error::other("the update never returned: the inline merge waits on its own lock")
    })??;
    assert!(segs <= crate::store::MAX_SEGMENTS, "{segs} segments: no merge ran");
    Ok(())
}

/// Class guard: a write escapes `.git/gitterrace/`. Every user-data write runs beside a
/// repository and a working tree, and afterwards nothing outside the cache folder changed, so a
/// future cache file written relative to the worktree or the git dir fails here.
#[test]
fn every_user_data_write_stays_under_the_cache_dir() -> io::Result<()> {
    fn tree(root: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) -> io::Result<()> {
        for e in std::fs::read_dir(root)? {
            let p = e?.path();
            if p.is_dir() {
                tree(&p, out)?;
            } else {
                out.push((p.display().to_string(), std::fs::read(&p)?));
            }
        }
        Ok(())
    }
    let tmp = tempfile::tempdir()?;
    let work = tmp.path().join("work");
    let git = work.join(".git");
    std::fs::create_dir_all(git.join("refs"))?;
    std::fs::write(work.join("a.txt"), "one\n")?;
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n")?;
    let cache = git.join("gitterrace");
    let idx = Index::open_dir(cache.clone())?;
    let outside = |extra: &mut Vec<(String, Vec<u8>)>| -> io::Result<()> {
        let mut all = Vec::new();
        tree(tmp.path(), &mut all)?;
        let inside = cache.display().to_string();
        all.retain(|(p, _)| !p.starts_with(&inside));
        *extra = all;
        Ok(())
    };
    let mut before = Vec::new();
    outside(&mut before)?;
    let file = gitterrace_git::SnapFile {
        path: "a.txt".into(),
        hash: gitterrace_git::content_id(b"one\n"),
        text: Some(b"one\n".to_vec()),
    };
    idx.checkpoint_add(&[file], "abc", true, "x", 1 << 10, 5)?;
    idx.viewed_set(&[("../../escape".to_owned(), "h".to_owned())], true, 5)?;
    idx.rebuild()?;
    let mut after = Vec::new();
    outside(&mut after)?;
    assert_eq!(before, after, "something outside .git/gitterrace changed");
    assert!(!tmp.path().join("escape").exists());
    assert!(cache.join("checkpoints").is_file() && cache.join("viewed").is_file());
    Ok(())
}
