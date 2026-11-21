//! The commits of a rev that the indexed set never reached: read from git on demand, at most
//! a capped number, so a view of a side branch costs its divergence, not the history. Strings
//! new to the set get ids after the set's own, so ids of indexed strings never change.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;

use crate::feed::{CommitRow, FileRow, IdentId, InsightsSource, LabelId, PathId, Rank, Sums};
use crate::{CommitRecord, CommitSource};

/// Most commits one view reads from git by default (`limits.unindexed_commits`). Past it the view
/// asks for a reindex instead.
pub const FRESH_CAP: usize = 2000;

pub(crate) enum Walk {
    /// The unindexed commits (canonical order among themselves) and the indexed ranks where
    /// the walk met the set.
    Done(Fresh, Vec<Rank>),
    /// More than the cap: how many were read before stopping.
    TooFar(usize),
}

pub(crate) struct FreshRow {
    pub(crate) row: CommitRow,
    pub(crate) id: Arc<str>,
    pub(crate) parents: Vec<Arc<str>>,
    pub(crate) files: Vec<FileRow>,
}

#[derive(Default)]
pub(crate) struct Fresh {
    pub(crate) rows: Vec<FreshRow>,
    pub(crate) ids: HashMap<Arc<str>, usize>,
    /// Strings the set does not hold, id `base count + position`.
    pub(crate) paths: Vec<Arc<str>>,
    pub(crate) idents: Vec<(Arc<str>, Arc<str>)>,
    pub(crate) labels: Vec<Arc<str>>,
}

/// Reads `tip` and its ancestors from git until each path meets the set of `base`, at most
/// `cap` commits. `rev` names the mailmap to apply to new identities.
pub(crate) fn walk(
    base: &dyn InsightsSource,
    src: &dyn CommitSource,
    rev: &str,
    tip: &str,
    cap: usize,
) -> io::Result<Walk> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut todo = vec![tip.to_owned()];
    let mut met = Vec::new();
    let mut recs: Vec<CommitRecord> = Vec::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        // Absence reads the commit from git, so an error must not pass for it.
        if let Some(r) = base.find_commit(&id).map_err(crate::feed::SourceError::io)? {
            met.push(r);
            continue;
        }
        if recs.len() >= cap {
            return Ok(Walk::TooFar(recs.len()));
        }
        let r = src.record(&id)?;
        todo.extend(r.parents.iter().rev().cloned());
        recs.push(r);
    }
    // The unindexed commits all come after the indexed part, which none of them is an
    // ancestor of, so they sort alone by (author time, id).
    let nodes: Vec<(i64, &str, Vec<&str>)> = recs
        .iter()
        .map(|r| (r.time, r.id.as_str(), r.parents.iter().map(String::as_str).collect()))
        .collect();
    let order = gitterrace_git::topo_sort(&nodes);
    let mut f = Fresh::default();
    let latest = crate::wall_clock().saturating_add(crate::insights::DAY);
    let first = u32::try_from(base.len()).unwrap_or(u32::MAX);
    let (shown, raw_ix) = crate::source::mapped_pairs(src, rev, &recs)?;
    let mut ident_of: Vec<Option<IdentId>> = vec![None; shown.len()];
    for (k, i) in order.into_iter().enumerate() {
        let r = &recs[i];
        let rank = Rank(first.saturating_add(u32::try_from(k).unwrap_or(u32::MAX)));
        let ix = raw_ix.get(&(r.author.clone(), r.email.clone())).copied().unwrap_or(0);
        let ident = *ident_of[ix].get_or_insert_with(|| f.ident_id(base, &shown[ix]));
        let kind = r.kind.as_deref().map(|k| f.label_id(base, k));
        let scope = r.scope.as_deref().map(|s| f.label_id(base, s));
        let mut sums = Sums::default();
        let mut files = Vec::with_capacity(r.files.len());
        for s in &r.files {
            let row = FileRow {
                path: f.path_id(base, &s.path),
                old: s.old.as_deref().map(|o| f.path_id(base, o)),
                add: s.additions,
                del: s.deletions,
                lines: s.lines,
                rewritten: s.rewritten,
                code: s.code.map(|c| (c.additions, c.deletions, c.rewritten)),
                binary: s.binary,
                deleted: s.deleted,
            };
            sums.add_row(&row).ok_or_else(|| io::Error::other(crate::seg::agg::SUM_OVER))?;
            files.push(row);
        }
        let row = CommitRow {
            rank,
            ident,
            time: crate::plausible_time(r.time, latest),
            offset: r.offset,
            merge: r.merge,
            kind,
            scope,
            files: u32::try_from(files.len()).unwrap_or(u32::MAX),
            sums,
        };
        let id: Arc<str> = r.id.as_str().into();
        f.ids.insert(Arc::clone(&id), k);
        let parents = r.parents.iter().map(|p| p.as_str().into()).collect();
        f.rows.push(FreshRow { row, id, parents, files });
    }
    Ok(Walk::Done(f, met))
}

/// Labels have no count on the seam, so new ones start past the last id the set names.
pub(crate) fn label_count(base: &dyn InsightsSource) -> usize {
    let mut n = 0u32;
    while base.label(LabelId(n)).is_some() {
        n += 1;
    }
    n as usize
}

fn next_id(base: usize, extra: usize) -> u32 {
    u32::try_from(base + extra).unwrap_or(u32::MAX)
}

impl Fresh {
    fn path_id(&mut self, base: &dyn InsightsSource, p: &str) -> PathId {
        if let Some(id) = base.path_id(p) {
            return id;
        }
        if let Some(i) = self.paths.iter().position(|q| &**q == p) {
            return PathId(next_id(base.paths(), i));
        }
        self.paths.push(p.into());
        PathId(next_id(base.paths(), self.paths.len() - 1))
    }

    fn label_id(&mut self, base: &dyn InsightsSource, l: &str) -> LabelId {
        if let Some(id) = base.label_id(l) {
            return id;
        }
        if let Some(i) = self.labels.iter().position(|q| &**q == l) {
            return LabelId(next_id(label_count(base), i));
        }
        self.labels.push(l.into());
        LabelId(next_id(label_count(base), self.labels.len() - 1))
    }

    /// A shown pair the set already serves keeps that id (equal strings, the seam's rule).
    fn ident_id(&mut self, base: &dyn InsightsSource, shown: &(String, String)) -> IdentId {
        let n = base.idents();
        for i in 0..n {
            let id = IdentId(next_id(i, 0));
            if base.ident(id).is_some_and(|(a, b)| *a == *shown.0 && *b == *shown.1) {
                return id;
            }
        }
        if let Some(i) = self.idents.iter().position(|(a, b)| **a == *shown.0 && **b == *shown.1) {
            return IdentId(next_id(n, i));
        }
        self.idents.push((shown.0.as_str().into(), shown.1.as_str().into()));
        IdentId(next_id(n, self.idents.len() - 1))
    }
}
