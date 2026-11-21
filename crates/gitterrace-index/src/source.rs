use std::collections::HashMap;
use std::io;

use gitterrace_git::Repo;

use crate::{CodeStat, CommitRecord, FileStat, conventional};

/// What the indexer needs from a repository. A seam so the store is testable without git.
pub trait CommitSource {
    /// Commit ids reachable from `rev`, oldest first.
    fn rev_list(&self, rev: &str) -> io::Result<Vec<String>>;

    /// The commit id `rev` names, `None` when the source cannot say (test doubles).
    fn resolve(&self, _rev: &str) -> Option<String> {
        None
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord>;

    /// Builds records for many commits, in the order of `ids`. A commit that cannot be read is
    /// left out, never returned with empty stats: the index then keeps it pending and retries
    /// it. Sources that can do the work in parallel override this.
    fn records(&self, ids: &[String]) -> io::Result<Vec<CommitRecord>> {
        ids.iter().map(|id| self.record(id)).collect()
    }

    /// Builds records for `ids` and hands each to `emit` in the order of `ids`, skipping the
    /// ones that cannot be read like [`CommitSource::records`]. `done` reports how many of
    /// `ids` are finished, read or skipped. Sources that can stream work without batch
    /// barriers override this; the default builds `chunk` commits at a time.
    fn stream_records(
        &self,
        ids: &[String],
        chunk: usize,
        emit: &mut dyn FnMut(CommitRecord) -> io::Result<()>,
        done: &mut dyn FnMut(usize),
    ) -> io::Result<()> {
        let mut n = 0;
        for part in ids.chunks(chunk.max(1)) {
            for record in self.records(part)? {
                emit(record)?;
            }
            n += part.len();
            done(n);
        }
        Ok(())
    }

    /// Commits whose parents are missing (a shallow clone's boundary). Their records are diffed
    /// as roots and rebuilt when the boundary moves past them.
    fn shallow_ids(&self) -> Vec<String> {
        Vec::new()
    }

    /// Hands the commit search text of each of `ids` to `emit`, in order, for the message
    /// column. `Ok(false)` when this source has none (a test double): the column is then not
    /// built. A commit that cannot be read fails the whole call: a column with a hole would
    /// give wrong results.
    fn stream_texts(
        &self,
        _ids: &[String],
        _emit: &mut dyn FnMut(gitterrace_git::CommitText) -> io::Result<()>,
    ) -> io::Result<bool> {
        Ok(false)
    }

    /// Maps `(name, email)` pairs as the tip of `rev` says (its `.mailmap`). Records are stored
    /// with the identities as written, so this runs on every read.
    fn map_identities(
        &self,
        _rev: &str,
        pairs: &[(String, String)],
    ) -> io::Result<Vec<(String, String)>> {
        Ok(pairs.to_vec())
    }
}

/// The mapped pairs and each raw pair's position among them.
pub(crate) type MappedPairs = (Vec<(String, String)>, HashMap<(String, String), usize>);

/// The mapped form of every distinct (name, email) pair of `records`, asked of the source
/// once, and each pair's position in that list.
pub(crate) fn mapped_pairs(
    source: &dyn CommitSource,
    rev: &str,
    records: &[CommitRecord],
) -> io::Result<MappedPairs> {
    let mut slot: HashMap<(String, String), usize> = HashMap::new();
    let mut pairs = Vec::new();
    for r in records {
        let key = (r.author.clone(), r.email.clone());
        if !slot.contains_key(&key) {
            slot.insert(key.clone(), pairs.len());
            pairs.push(key);
        }
    }
    let mapped = source.map_identities(rev, &pairs)?;
    if mapped.len() != pairs.len() {
        return Err(wrong_pair_count());
    }
    Ok((mapped, slot))
}

/// The error for a source that answers an identity map with another number of pairs.
pub(crate) fn wrong_pair_count() -> io::Error {
    io::Error::other("identity map returned the wrong number of pairs")
}

fn file_stats(diffs: Vec<gitterrace_git::FileDiff>) -> Vec<FileStat> {
    diffs
        .into_iter()
        .map(|f| {
            let raw = (f.additions, f.deletions, f.rewritten);
            let code = (f.code.additions, f.code.deletions, f.code.rewritten);
            FileStat {
                path: f.path.into(),
                additions: f.additions,
                deletions: f.deletions,
                lines: f.lines,
                rewritten: f.rewritten,
                code: (code != raw && !f.binary).then_some(CodeStat {
                    additions: f.code.additions,
                    deletions: f.code.deletions,
                    rewritten: f.code.rewritten,
                }),
                old: f.old_path.map(Into::into),
                binary: f.binary,
                deleted: f.kind == gitterrace_git::ChangeKind::Deleted,
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
        // The canonical order, not the walk's: ordinals and string ids follow it.
        gitterrace_git::canonical_order(self, rev).map_err(io::Error::other)
    }

    fn resolve(&self, rev: &str) -> Option<String> {
        Self::resolve(self, rev).ok()
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        let mut all = self.records(&[id.to_owned()])?;
        all.pop().ok_or_else(|| io::Error::other(format!("commit {id} not found")))
    }

    fn records(&self, ids: &[String]) -> io::Result<Vec<CommitRecord>> {
        let diffs = self.numstat_many(ids).map_err(io::Error::other)?;
        let summaries = self.raw_summaries(ids).map_err(io::Error::other)?;
        Ok(summaries
            .into_iter()
            .zip(diffs)
            .filter_map(|(summary, d)| d.map(|d| build(summary, file_stats(d))))
            .collect())
    }

    fn stream_records(
        &self,
        ids: &[String],
        chunk: usize,
        emit: &mut dyn FnMut(CommitRecord) -> io::Result<()>,
        done: &mut dyn FnMut(usize),
    ) -> io::Result<()> {
        let chunk = chunk.max(1);
        self.numstat_stream(ids, |i, commit| {
            if let Some((summary, files)) = commit {
                emit(build(summary, file_stats(files)))
                    .map_err(|e| gitterrace_git::Error::Git(e.to_string()))?;
            }
            if (i + 1) % chunk == 0 || i + 1 == ids.len() {
                done(i + 1);
            }
            Ok(())
        })
        .map_err(io::Error::other)
    }

    fn shallow_ids(&self) -> Vec<String> {
        Self::shallow_ids(self)
    }

    fn stream_texts(
        &self,
        ids: &[String],
        emit: &mut dyn FnMut(gitterrace_git::CommitText) -> io::Result<()>,
    ) -> io::Result<bool> {
        self.stream_commit_texts(ids, |t| {
            emit(t).map_err(|e| gitterrace_git::Error::Git(e.to_string()))
        })
        .map_err(io::Error::other)?;
        Ok(true)
    }

    fn map_identities(
        &self,
        rev: &str,
        pairs: &[(String, String)],
    ) -> io::Result<Vec<(String, String)>> {
        Self::map_identities(self, rev, pairs).map_err(io::Error::other)
    }
}
