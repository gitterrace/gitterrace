use std::io;

use gitterrace_git::Repo;

use crate::{CommitRecord, FileStat, conventional};

/// What the indexer needs from a repository. A seam so the store is testable without git.
pub trait CommitSource {
    /// Commit ids reachable from `rev`, oldest first.
    fn rev_list(&self, rev: &str) -> io::Result<Vec<String>>;

    /// Builds the record for one commit.
    fn record(&self, id: &str) -> io::Result<CommitRecord>;
}

impl CommitSource for Repo {
    fn rev_list(&self, rev: &str) -> io::Result<Vec<String>> {
        Repo::rev_list(self, rev).map_err(io::Error::other)
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        let summary = Repo::log(self, id, None, 0, 1)
            .map_err(io::Error::other)?
            .into_iter()
            .next()
            .ok_or_else(|| io::Error::other(format!("commit {id} not found")))?;
        let files = Repo::numstat(self, id)
            .map_err(io::Error::other)?
            .into_iter()
            .map(|f| FileStat {
                path: f.path,
                additions: f.additions,
                deletions: f.deletions,
            })
            .collect();
        let (kind, scope) = conventional::parse_subject(&summary.subject);
        Ok(CommitRecord {
            id: summary.id,
            author: summary.author.name,
            email: summary.author.email,
            time: summary.author.time,
            offset: summary.author.offset,
            merge: summary.parents.len() > 1,
            kind,
            scope,
            files,
        })
    }
}
