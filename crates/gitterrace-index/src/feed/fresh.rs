//! The commits of a rev that the indexed set never reached: read from git on demand, at most
//! a capped number, so a view of a side branch costs its divergence, not the history. Strings
//! new to the set get ids after the set's own, so ids of indexed strings never change.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::io;
use std::sync::Arc;

use crate::feed::{CommitRow, FileRow, IdentId, InsightsSource, LabelId, PathId, Rank, Sums};
use crate::{CommitRecord, CommitSource};

/// Most commits one view reads from git by default (`limits.fresh_commits`). Past it the view
/// asks for a reindex instead.
pub const FRESH_CAP: usize = 2000;

/// What the bounded read found.
pub(crate) enum Walk {
    /// The unindexed commits (canonical order among themselves) and the indexed ranks where
    /// the walk met the set.
    Done(Fresh, Vec<Rank>),
    /// More than the cap: how many were read before stopping.
    TooFar(usize),
}

/// One unindexed commit: its columns, id, parent ids and file rows.
pub(crate) struct FreshRow {
    pub(crate) row: CommitRow,
    pub(crate) id: Arc<str>,
    pub(crate) parents: Vec<Arc<str>>,
    pub(crate) files: Vec<FileRow>,
}

/// Unindexed commits of one view, oldest first, with ids for their strings.
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
        if let Some(r) = base.find_commit(&id) {
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
    let order = topo(&recs);
    let mut f = Fresh::default();
    let latest = crate::wall_clock().saturating_add(crate::insights::DAY);
    let first = u32::try_from(base.len()).unwrap_or(u32::MAX);
    let mut raw: Vec<(String, String)> = Vec::new();
    let mut raw_ix: HashMap<(String, String), usize> = HashMap::new();
    for r in &recs {
        let key = (r.author.clone(), r.email.clone());
        if !raw_ix.contains_key(&key) {
            raw_ix.insert(key.clone(), raw.len());
            raw.push(key);
        }
    }
    let shown = src.map_identities(rev, &raw)?;
    if shown.len() != raw.len() {
        return Err(io::Error::other(
            "identity map returned the wrong number of pairs",
        ));
    }
    let mut ident_of: Vec<Option<IdentId>> = vec![None; raw.len()];
    for (k, i) in order.into_iter().enumerate() {
        let r = &recs[i];
        let rank = Rank(first.saturating_add(u32::try_from(k).unwrap_or(u32::MAX)));
        let ix = raw_ix
            .get(&(r.author.clone(), r.email.clone()))
            .copied()
            .unwrap_or(0);
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
            };
            sums.add_row(&row);
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
        f.rows.push(FreshRow {
            row,
            id,
            parents,
            files,
        });
    }
    Ok(Walk::Done(f, met))
}

/// Kahn over the unindexed commits alone, ready ones by (author time, id): they all come after
/// the indexed part, which none of them is an ancestor of.
fn topo(recs: &[CommitRecord]) -> Vec<usize> {
    let pos: HashMap<&str, usize> = recs
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.as_str(), i))
        .collect();
    let mut waiting = vec![0usize; recs.len()];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); recs.len()];
    for (i, r) in recs.iter().enumerate() {
        for p in r.parents.iter().filter_map(|p| pos.get(p.as_str())) {
            waiting[i] += 1;
            children[*p].push(i);
        }
    }
    let mut ready: BinaryHeap<Reverse<(i64, &str, usize)>> = waiting
        .iter()
        .enumerate()
        .filter(|(_, w)| **w == 0)
        .map(|(i, _)| Reverse((recs[i].time, recs[i].id.as_str(), i)))
        .collect();
    let mut out = Vec::with_capacity(recs.len());
    while let Some(Reverse((_, _, i))) = ready.pop() {
        out.push(i);
        for &c in &children[i] {
            waiting[c] -= 1;
            if waiting[c] == 0 {
                ready.push(Reverse((recs[c].time, recs[c].id.as_str(), c)));
            }
        }
    }
    out
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
        if let Some(id) = base.find_path(p) {
            return id;
        }
        if let Some(i) = self.paths.iter().position(|q| &**q == p) {
            return PathId(next_id(base.paths(), i));
        }
        self.paths.push(p.into());
        PathId(next_id(base.paths(), self.paths.len() - 1))
    }

    fn label_id(&mut self, base: &dyn InsightsSource, l: &str) -> LabelId {
        if let Some(id) = base.find_label(l) {
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
            if base
                .ident(id)
                .is_some_and(|(a, b)| *a == *shown.0 && *b == *shown.1)
            {
                return id;
            }
        }
        if let Some(i) = self
            .idents
            .iter()
            .position(|(a, b)| **a == *shown.0 && **b == *shown.1)
        {
            return IdentId(next_id(n, i));
        }
        self.idents
            .push((shown.0.as_str().into(), shown.1.as_str().into()));
        IdentId(next_id(n, self.idents.len() - 1))
    }
}
