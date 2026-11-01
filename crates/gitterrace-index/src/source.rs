use std::io;

use gitterrace_git::Repo;

use crate::{CodeStat, CommitRecord, FileStat, conventional};

/// What the indexer needs from a repository. A seam so the store is testable without git.
pub trait CommitSource {
    /// Commit ids reachable from `rev`, oldest first.
    fn rev_list(&self, rev: &str) -> io::Result<Vec<String>>;

    /// Builds the record for one commit.
    fn record(&self, id: &str) -> io::Result<CommitRecord>;

    /// Builds records for many commits, in the order of `ids`. Sources that can do the work in
    /// parallel override this.
    fn records(&self, ids: &[String]) -> io::Result<Vec<CommitRecord>> {
        ids.iter().map(|id| self.record(id)).collect()
    }
}

fn file_stats(diffs: Vec<gitterrace_git::FileDiff>) -> Vec<FileStat> {
    diffs
        .into_iter()
        .map(|f| {
            let raw = (f.additions, f.deletions, f.rewritten);
            let code = (f.code.additions, f.code.deletions, f.code.rewritten);
            FileStat {
                path: f.path,
                additions: f.additions,
                deletions: f.deletions,
                lines: f.lines,
                rewritten: f.rewritten,
                code: (code != raw && !f.binary).then_some(CodeStat {
                    additions: f.code.additions,
                    deletions: f.code.deletions,
                    rewritten: f.code.rewritten,
                }),
                old: f.old_path,
                binary: f.binary,
            }
        })
        .collect()
}

fn build(summary: gitterrace_git::CommitSummary, files: Vec<FileStat>) -> CommitRecord {
    let (kind, scope) = conventional::parse_subject(&summary.subject);
    CommitRecord {
        id: summary.id,
        author: summary.author.name,
        email: summary.author.email,
        time: summary.author.time,
        offset: summary.author.offset,
        merge: summary.parents.len() > 1,
        parents: summary.parents,
        kind,
        scope,
        files,
    }
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
        // Unreadable objects (a shallow boundary) cost the file stats, not the record.
        let files = file_stats(Repo::numstat(self, id).unwrap_or_default());
        Ok(build(summary, files))
    }

    fn records(&self, ids: &[String]) -> io::Result<Vec<CommitRecord>> {
        let diffs = self.numstat_many(ids).map_err(io::Error::other)?;
        ids.iter()
            .zip(diffs)
            .map(|(id, d)| {
                let summary = Repo::log(self, id, None, 0, 1)
                    .map_err(io::Error::other)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| io::Error::other(format!("commit {id} not found")))?;
                Ok(build(summary, file_stats(d)))
            })
            .collect()
    }
}
