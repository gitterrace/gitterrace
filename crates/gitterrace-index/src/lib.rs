//! Incremental insights cache in `.git/gitterrace/`. One record per commit, appended;
//! statistics are computed from the records on demand by [`Insights`].

mod cache;
mod conventional;
mod insights;
mod source;
mod store;

use std::io;
use std::path::PathBuf;

use gitterrace_git::Repo;

/// Re-exported so the cards crate shares the one date rule without a new dependency.
pub use gitterrace_git::civil_from_days;
pub use insights::*;
pub use source::CommitSource;
pub use store::FORMAT_VERSION;

/// Every file the cache keeps in `.git/gitterrace/`, with what it holds. The architecture page is
/// rendered from this, and a test fails when the folder holds a file not listed here.
pub const CACHE_FILES: [(&str, &str); 4] = [
    (
        store::VERSION_FILE,
        "The cache format. A mismatch deletes the cache and rebuilds it. The format is never migrated.",
    ),
    (
        store::COMMITS_FILE,
        "One record per commit, appended as the history is walked. Unreachable records are ignored, so rewritten history needs no cleanup, and a torn last line is dropped.",
    ),
    (
        store::PATHS_FILE,
        "Every file path any record mentions, one JSON string per line. Records refer to a path by its line number, so a path is stored once however many commits touch it.",
    ),
    (
        insights::CACHE_FILE,
        "Line counts per blob id for the language chart. A blob never changes, so the counts never go stale.",
    ),
];

/// Commits built per batch. Every batch pays a fixed cost per worker (the diff attribute setup
/// unpacks the HEAD tree, 40% of the time at 256 on a 50k-file repository), so it is large
/// enough to amortize that and still small enough for steady progress and bounded memory.
const BATCH: usize = 2048;

/// Per-commit facts the stats pages are built from.
#[derive(Debug, Clone, Default)]
pub struct CommitRecord {
    pub id: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub offset: i32,
    pub merge: bool,
    /// Parent ids, first parent first. Lets history be followed along the first-parent chain.
    pub parents: Vec<String>,
    /// Conventional-commit type and scope, when the subject follows it.
    pub kind: Option<String>,
    pub scope: Option<String>,
    pub files: Vec<FileStat>,
}

/// One file touched by a commit, diffed against the first parent with rename detection.
#[derive(Debug, Clone, Default)]
pub struct FileStat {
    pub path: std::sync::Arc<str>,
    pub additions: u32,
    pub deletions: u32,
    /// Line count of the file after the commit (0 when deleted or binary).
    pub lines: u32,
    /// Lines in change blocks that both removed and added: the rework part of the change.
    pub rewritten: u32,
    /// Counts with whitespace-only and blank-line changes ignored. `None` when equal to the
    /// raw counts, which is nearly always.
    pub code: Option<CodeStat>,
    /// Previous path when the file was renamed in this commit.
    pub old: Option<std::sync::Arc<str>>,
    pub binary: bool,
}

/// Line counts that ignore whitespace-only differences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CodeStat {
    pub additions: u32,
    pub deletions: u32,
    pub rewritten: u32,
}

impl FileStat {
    /// Counts without whitespace noise, or the raw counts when nothing differs.
    pub fn code_stat(&self) -> CodeStat {
        self.code.unwrap_or(CodeStat {
            additions: self.additions,
            deletions: self.deletions,
            rewritten: self.rewritten,
        })
    }
}

/// Progress of the background build, read by the web layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

/// The cache for one repository.
pub struct Index {
    dir: PathBuf,
    /// Threads that parse the stored records; 0 means one per core, at most 8.
    threads: usize,
    /// The format of a cache written by a newer build. Such a cache is never read or written.
    newer: Option<String>,
}

impl Index {
    /// Opens or creates the cache under `repo.git_dir()/gitterrace`.
    pub fn open(repo: &Repo) -> io::Result<Self> {
        Self::open_dir(repo.git_dir().join("gitterrace"))
    }

    /// Opens or creates the cache in `dir`. A format version mismatch discards old records.
    pub fn open_dir(dir: PathBuf) -> io::Result<Self> {
        let newer = store::ensure_dir(&dir)?;
        Ok(Self {
            dir,
            threads: 0,
            newer,
        })
    }

    /// Sets the threads that parse stored records; 0 means one per core, at most 8.
    #[must_use]
    pub fn with_threads(mut self, threads: usize) -> Self {
        self.threads = threads;
        self
    }

    /// The cache format found on disk when a newer build wrote it. Then [`Index::update`] and
    /// [`Index::records`] fail with an explanation and leave the files alone.
    pub fn newer_format(&self) -> Option<&str> {
        self.newer.as_deref()
    }

    fn writable(&self) -> io::Result<()> {
        store::check_writable(&self.dir)
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Size in bytes of the cache file. It changes whenever records are added, so a cache of
    /// parsed records can key on it.
    pub fn generation(&self) -> u64 {
        store::file_len(&self.dir)
    }

    /// Indexes commits reachable from `rev` that are not yet cached.
    pub fn update(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        progress: &dyn Fn(Progress),
    ) -> io::Result<()> {
        self.writable()?;
        let ids = source.rev_list(rev)?;
        let (known, valid) = store::stored_ids(&self.dir)?;
        let mut seen = known;
        let todo: Vec<&String> = ids.iter().filter(|id| seen.insert((*id).clone())).collect();
        let total = todo.len();
        progress(Progress { done: 0, total });
        let mut out = store::Appender::open(&self.dir, valid)?;
        let mut done = 0;
        for chunk in todo.chunks(BATCH) {
            let ids: Vec<String> = chunk.iter().map(|id| (*id).clone()).collect();
            for record in source.records(&ids)? {
                out.push(&record)?;
            }
            done += chunk.len();
            progress(Progress { done, total });
        }
        out.flush()
    }

    /// Commits reachable from `rev` that are not cached yet. Zero means [`Index::records`] for
    /// `rev` is complete; insights for a branch other than HEAD call [`Index::update`] first.
    pub fn pending(&self, source: &dyn CommitSource, rev: &str) -> io::Result<usize> {
        let ids = source.rev_list(rev)?;
        let (known, _) = store::stored_ids(&self.dir)?;
        Ok(ids.iter().filter(|id| !known.contains(*id)).count())
    }

    /// All cached records reachable from `rev`, oldest first.
    pub fn records(&self, source: &dyn CommitSource, rev: &str) -> io::Result<Vec<CommitRecord>> {
        let order = source.rev_list(rev)?;
        let (records, _) = store::read_records(&self.dir, self.threads)?;
        Ok(store::select(records, &order))
    }
}

#[cfg(test)]
mod tests {
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
        let (known, _) = crate::store::stored_ids(idx.dir())?;
        assert_eq!(known.len(), 4000);
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
            out.push(&r)?;
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
}
