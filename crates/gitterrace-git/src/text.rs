//! What commit search reads per commit, for the index's message column, and the reads that
//! turn its matches back into summaries.

use std::sync::atomic::Ordering;

use gix::bstr::ByteSlice;

use crate::error::git;
use crate::ident;
use crate::repo::{INDEX_WORKER_CACHE_BYTES, NUMSTAT_WINDOW, Repo};
use crate::{CommitSummary, Result};

/// One commit as commit search sees it: the full message as the walk decodes it (lossy UTF-8),
/// the author as written (bytes, before mailmap), and what the newest-first walk orders by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitText {
    pub id: String,
    pub parents: Vec<String>,
    /// Committer seconds as the walk's queue reads them (0 when unparsable).
    pub committer_time: i64,
    pub author_name: Vec<u8>,
    pub author_email: Vec<u8>,
    pub message: String,
}

/// The `.mailmap` of one commit, applied to raw author bytes.
pub struct Mailmap(gix::mailmap::Snapshot);

impl Mailmap {
    /// `name <email>` after the mailmap, as commit search matches it.
    pub fn show(&self, name: &[u8], email: &[u8]) -> String {
        let sig =
            gix::actor::SignatureRef { name: name.as_bstr(), email: email.as_bstr(), time: "" };
        let got = self.0.resolve(sig);
        format!("{} <{}>", got.name.to_str_lossy(), got.email.to_str_lossy())
    }
}

impl Repo {
    /// [`CommitText`] of each of `ids`, handed to `emit` in order, read on all cores with a
    /// bounded window, so memory stays flat. Fails on the first commit that cannot be read: a
    /// column with a hole would give wrong results.
    pub fn stream_commit_texts(
        &self,
        ids: &[String],
        mut emit: impl FnMut(CommitText) -> Result<()>,
    ) -> Result<()> {
        crate::diff::stream(
            ids,
            self.worker_count(),
            NUMSTAT_WINDOW,
            || {
                let mut repo = self.repo();
                repo.object_cache_size(INDEX_WORKER_CACHE_BYTES);
                Ok(repo)
            },
            |repo, id| {
                let commit = repo.find_commit(self.full_id(repo, id)?).map_err(git)?;
                let author = commit.author().map_err(git)?;
                let committer_time = commit.committer().map_err(git)?.seconds();
                Ok(CommitText {
                    id: commit.id.to_string(),
                    parents: commit.parent_ids().map(|p| p.to_string()).collect(),
                    committer_time,
                    author_name: author.name.to_vec(),
                    author_email: author.email.to_vec(),
                    message: commit.message_raw().map_err(git)?.to_str_lossy().into_owned(),
                })
            },
            |t| t.message.len() + t.author_name.len() + t.author_email.len() + 128,
            |_, t| emit(t),
        )
    }

    /// The mailmap commit search applies for `rev` (its tip's `.mailmap`).
    pub fn mailmap_at(&self, rev: &str) -> Result<Mailmap> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        Ok(Mailmap(ident::mailmap(&repo, tip)))
    }

    /// Summaries of `ids` in order, identities through `rev`'s mailmap, exactly as
    /// [`crate::scan_commits`] builds them, so an index hit lists as the walk would.
    pub fn search_summaries(&self, rev: &str, ids: &[String]) -> Result<Vec<CommitSummary>> {
        let repo = self.repo();
        let tip = self.resolve_id(&repo, rev)?;
        let mailmap = ident::mailmap(&repo, tip);
        ids.iter()
            .map(|id| {
                let commit = repo.find_commit(self.full_id(&repo, id)?).map_err(git)?;
                self.count_commit_read();
                crate::repo::summary(&commit, &mailmap)
            })
            .collect()
    }

    /// The full message of each of `ids`, in order: what the log's message terms test.
    pub fn messages(&self, ids: &[String]) -> Result<Vec<String>> {
        let repo = self.repo();
        ids.iter()
            .map(|id| {
                let commit = repo.find_commit(self.full_id(&repo, id)?).map_err(git)?;
                self.count_commit_read();
                Ok(commit.message_raw().map_err(git)?.to_str_lossy().into_owned())
            })
            .collect()
    }

    /// Commits read by commit search through this handle so far: the walk and the summaries
    /// of index matches. Lets a test prove an index miss reads no commit.
    pub fn commits_read(&self) -> u64 {
        self.commits_read.load(Ordering::Relaxed)
    }

    pub(crate) fn count_commit_read(&self) {
        self.commits_read.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use crate::Repo;

    #[test]
    fn commit_texts_keep_raw_author_and_lossy_message() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = gitterrace_testkit::Fixture::init(dir.path())
            .write("a", "1")
            .commit("feat: one\n\nbody line");
        let repo = Repo::discover(f.path()).expect("repo");
        let head = repo.resolve("HEAD").expect("head");
        let mut t = Vec::new();
        repo.stream_commit_texts(std::slice::from_ref(&head), |x| {
            t.push(x);
            Ok(())
        })
        .expect("texts");
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, head);
        assert!(t[0].message.starts_with("feat: one\n\nbody line"));
        assert_eq!(t[0].parents, [] as [std::string::String; 0]);
        let s = repo.search_summaries("HEAD", &[head]).expect("summaries");
        assert_eq!(s[0].subject, "feat: one");
        assert_eq!(repo.commits_read(), 1);
    }
}
