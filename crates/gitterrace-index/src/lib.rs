//! Incremental stats cache in `.git/gitterrace/`. One record per commit, appended;
//! aggregates are computed from the records on demand.

mod aggregates;
mod conventional;
mod insights;
mod source;
mod store;

use std::io;
use std::path::PathBuf;

use gitterrace_git::Repo;

pub use aggregates::*;
pub use insights::*;
pub use source::CommitSource;

/// Commits built per batch: enough to keep all cores busy, small enough for steady progress.
const BATCH: usize = 256;

/// Per-commit facts the stats pages are built from.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CommitRecord {
    pub id: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub offset: i32,
    pub merge: bool,
    /// Parent ids, first parent first. Lets history be followed along the first-parent chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    /// Conventional-commit type and scope, when the subject follows it.
    pub kind: Option<String>,
    pub scope: Option<String>,
    pub files: Vec<FileStat>,
}

/// One file touched by a commit, diffed against the first parent with rename detection.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct FileStat {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    /// Line count of the file after the commit (0 when deleted or binary).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lines: u32,
    /// Lines in change blocks that both removed and added: the rework part of the change.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rewritten: u32,
    /// Counts with whitespace-only and blank-line changes ignored. `None` when equal to the
    /// raw counts, which is nearly always.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<CodeStat>,
    /// Previous path when the file was renamed in this commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
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

#[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if signature
fn is_zero(n: &u32) -> bool {
    *n == 0
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
}

impl Index {
    /// Opens or creates the cache under `repo.git_dir()/gitterrace`.
    pub fn open(repo: &Repo) -> io::Result<Self> {
        Self::open_dir(repo.git_dir().join("gitterrace"))
    }

    /// Opens or creates the cache in `dir`. A format version mismatch discards old records.
    pub fn open_dir(dir: PathBuf) -> io::Result<Self> {
        store::ensure_dir(&dir)?;
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Indexes commits reachable from `rev` that are not yet cached.
    pub fn update(&self, repo: &Repo, rev: &str, progress: &dyn Fn(Progress)) -> io::Result<()> {
        self.update_from(repo, rev, progress)
    }

    /// All cached records reachable from `rev`, oldest first.
    pub fn records(&self, repo: &Repo, rev: &str) -> io::Result<Vec<CommitRecord>> {
        self.records_from(repo, rev)
    }

    /// [`Index::update`] over any commit source.
    pub fn update_from(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        progress: &dyn Fn(Progress),
    ) -> io::Result<()> {
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

    /// [`Index::records`] over any commit source.
    pub fn records_from(
        &self,
        source: &dyn CommitSource,
        rev: &str,
    ) -> io::Result<Vec<CommitRecord>> {
        let order = source.rev_list(rev)?;
        let (records, _) = store::read_records(&self.dir)?;
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
    }

    impl Fake {
        fn new(ids: &[&str]) -> Self {
            Self {
                ids: RefCell::new(ids.iter().map(|s| (*s).to_owned()).collect()),
                built: Cell::new(0),
            }
        }
    }

    impl CommitSource for Fake {
        fn rev_list(&self, _rev: &str) -> io::Result<Vec<String>> {
            Ok(self.ids.borrow().clone())
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

    #[test]
    fn update_is_incremental_and_ordered() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let idx = Index::open_dir(dir.path().join("gitterrace"))?;
        let src = Fake::new(&["a", "b"]);
        idx.update_from(&src, "HEAD", &|_| {})?;
        src.ids.borrow_mut().push("c".into());
        let last = Cell::new(Progress::default());
        idx.update_from(&src, "HEAD", &|p| last.set(p))?;
        assert_eq!(src.built.get(), 3);
        assert_eq!(last.get(), Progress { done: 1, total: 1 });
        assert_eq!(ids(&idx.records_from(&src, "HEAD")?), ["a", "b", "c"]);
        Ok(())
    }

    #[test]
    fn unreachable_records_are_ignored() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let idx = Index::open_dir(dir.path().join("gitterrace"))?;
        let src = Fake::new(&["a", "b", "c"]);
        idx.update_from(&src, "HEAD", &|_| {})?;
        src.ids.replace(vec!["a".into(), "c".into()]);
        assert_eq!(ids(&idx.records_from(&src, "HEAD")?), ["a", "c"]);
        Ok(())
    }

    #[test]
    fn torn_last_line_is_dropped_and_rebuilt() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("gitterrace");
        let idx = Index::open_dir(path.clone())?;
        let src = Fake::new(&["a", "b"]);
        idx.update_from(&src, "HEAD", &|_| {})?;
        let file = path.join("commits.jsonl");
        let mut text = std::fs::read_to_string(&file)?;
        text.pop();
        text.truncate(text.len() - 5);
        std::fs::write(&file, text)?;
        assert_eq!(ids(&idx.records_from(&src, "HEAD")?), ["a"]);
        idx.update_from(&src, "HEAD", &|_| {})?;
        assert_eq!(ids(&idx.records_from(&src, "HEAD")?), ["a", "b"]);
        assert_eq!(std::fs::read_to_string(&file)?.lines().count(), 2);
        Ok(())
    }

    #[test]
    fn version_mismatch_rebuilds() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("gitterrace");
        let src = Fake::new(&["a"]);
        Index::open_dir(path.clone())?.update_from(&src, "HEAD", &|_| {})?;
        std::fs::write(path.join("VERSION"), "0\n")?;
        let idx = Index::open_dir(path.clone())?;
        assert!(idx.records_from(&src, "HEAD")?.is_empty());
        idx.update_from(&src, "HEAD", &|_| {})?;
        assert_eq!(src.built.get(), 2);
        assert_eq!(
            std::fs::read_to_string(path.join("VERSION"))?.trim(),
            crate::store::FORMAT_VERSION
        );
        Ok(())
    }
}
