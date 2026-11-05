//! Merge of segments into one, renumbered into the canonical order.
//! Commit ordinals map through each segment's rank array (local ordinal to position in the
//! order). String ids map through a renumbering by canonical first use, taken from the
//! commits' own ids walked in the new order. The result equals a one-step build, so the
//! bytes equal after encoding.

use std::collections::BTreeMap;

use crate::seg::agg::{Aggregates, AuthorDay, Interner, PathAuthor, Strings};
use crate::seg::postings::{Edge, Hit, Postings};

/// One segment's tables as the merge reads them.
pub(crate) struct Part<'a> {
    pub agg: &'a Aggregates,
    pub postings: &'a Postings,
    /// Position of each local ordinal in the merged order.
    pub rank: &'a [u32],
}

/// Old global string id to new id, by canonical first use.
#[derive(Debug, Default)]
pub(crate) struct Renumber {
    pub paths: Vec<u32>,
    pub identities: Vec<u32>,
    pub strings: Strings,
}

impl Renumber {
    /// `uses` yields, in the merged canonical order, each commit's identity id and its rows'
    /// ids (old name before new for a rename), all in the old numbering.
    pub(crate) fn new<'u>(old: &Strings, uses: impl IntoIterator<Item = (u32, &'u [u32])>) -> Self {
        let mut n = Self {
            paths: vec![u32::MAX; old.paths.list.len()],
            identities: vec![u32::MAX; old.identities.list.len()],
            strings: Strings::default(),
        };
        for (who, paths) in uses {
            give(
                &mut n.identities,
                &mut n.strings.identities,
                &old.identities,
                who,
            );
            for &p in paths {
                give(&mut n.paths, &mut n.strings.paths, &old.paths, p);
            }
        }
        n
    }

    fn path(&self, p: u32) -> u32 {
        self.paths.get(p as usize).copied().unwrap_or(u32::MAX)
    }

    fn who(&self, w: u32) -> u32 {
        self.identities.get(w as usize).copied().unwrap_or(u32::MAX)
    }
}

fn give(map: &mut [u32], into: &mut Interner, old: &Interner, id: u32) {
    if let Some(slot) = map.get_mut(id as usize)
        && *slot == u32::MAX
        && let Some(s) = old.list.get(id as usize)
    {
        *slot = into.id(s);
    }
}

/// Aggregates summed over the parts, keys renumbered and re-sorted.
pub(crate) fn aggregates(parts: &[Part<'_>], n: &Renumber) -> Aggregates {
    let mut out = Aggregates::default();
    for part in parts {
        for (&(day, w), v) in &part.agg.author_day {
            let e: &mut AuthorDay = out.author_day.entry((day, n.who(w))).or_default();
            e.commits += v.commits;
            e.lines.merge(&v.lines);
        }
        for (&(p, w), v) in &part.agg.path_author {
            let e: &mut PathAuthor = out.path_author.entry((n.path(p), n.who(w))).or_default();
            e.touches += v.touches;
            e.lines.merge(&v.lines);
        }
    }
    out
}

/// A per-id table, each list sorted. Ids past `len` cannot occur (the renumbering is dense).
fn dense<T: Ord + Clone>(m: BTreeMap<u32, Vec<T>>, len: usize) -> Vec<Vec<T>> {
    let mut out = vec![Vec::new(); len];
    for (k, mut v) in m {
        v.sort_unstable();
        if let Some(slot) = out.get_mut(k as usize) {
            *slot = v;
        }
    }
    out
}

/// Postings of the parts with ordinals ranked and ids renumbered.
pub(crate) fn postings(parts: &[Part<'_>], n: &Renumber) -> Postings {
    let mut by_path: BTreeMap<u32, Vec<Hit>> = BTreeMap::new();
    let mut by_author: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    let mut edges = Vec::new();
    let rank = |r: &[u32], o: u32| r.get(o as usize).copied().unwrap_or(u32::MAX);
    for part in parts {
        for (p, hits) in part.postings.by_path.iter().enumerate() {
            let p = n.path(u32::try_from(p).unwrap_or(u32::MAX));
            let v = by_path.entry(p).or_default();
            v.extend(hits.iter().map(|h| Hit {
                ordinal: rank(part.rank, h.ordinal),
                row: h.row,
            }));
        }
        for (w, ords) in part.postings.by_author.iter().enumerate() {
            let w = n.who(u32::try_from(w).unwrap_or(u32::MAX));
            let v = by_author.entry(w).or_default();
            v.extend(ords.iter().map(|&o| rank(part.rank, o)));
        }
        edges.extend(part.postings.edges.iter().map(|e| Edge {
            ordinal: rank(part.rank, e.ordinal),
            row: e.row,
            old: n.path(e.old),
            new: n.path(e.new),
        }));
    }
    edges.sort_unstable();
    Postings::from_parts(
        dense(by_path, n.strings.paths.list.len()),
        dense(by_author, n.strings.identities.list.len()),
        edges,
    )
}

#[cfg(test)]
mod tests {
    use super::{Part, Renumber, aggregates, postings};
    use crate::CommitRecord;
    use crate::insights::fixtures::{file, rec};
    use crate::seg::agg::{Aggregates, Strings};
    use crate::seg::postings::Postings;

    fn renamed(new: &str, old: &str) -> crate::FileStat {
        let mut f = file(new, 1, 1, 5);
        f.old = Some(old.into());
        f
    }

    fn history() -> Vec<CommitRecord> {
        vec![
            rec("1", "Ann", 0, vec![file("b", 1, 0, 1), file("a", 1, 0, 1)]),
            rec("2", "Bob", 1, vec![renamed("c", "a")]),
            rec("3", "Cy", 2, vec![file("d", 2, 0, 2), file("b", 1, 0, 2)]),
            rec("4", "Ann", 40, vec![renamed("e", "d"), file("f", 1, 1, 2)]),
            rec("5", "Dee", 41, vec![file("a", 3, 0, 3)]),
        ]
    }

    /// A segment built over `recs`, ids global against `s`, as an unmerged set numbers them.
    fn segment(s: &mut Strings, recs: &[CommitRecord]) -> (Aggregates, Postings, Vec<Vec<u32>>) {
        let mut a = Aggregates::default();
        let mut p = Postings::default();
        let mut uses = Vec::new();
        for r in recs {
            let (who, paths) = s.ids(r);
            let olds = crate::seg::canon::olds(s, r);
            a.push(r, who, &paths, &olds);
            p.push(s, r, who, &paths);
            let mut u = vec![who];
            for (f, &id) in r.files.iter().zip(&paths) {
                u.push(id);
                if let Some(o) = f.old.as_ref().and_then(|o| s.paths.get(o)) {
                    u.push(o);
                }
            }
            uses.push(u);
        }
        (a, p, uses)
    }

    /// Step one, a merged-in branch dated earlier, then merge: equal to the one-step build,
    /// including string ids, which the branch's first use reorders.
    #[test]
    fn two_step_then_merge_equals_one_step() {
        let all = history();
        let (one_s, one_a, one_p) = {
            let mut s = Strings::default();
            let (a, p, _) = segment(&mut s, &all);
            (s, a, p)
        };
        // Segment 0 holds commits 1, 2, 4; segment 1 brings 3 and 5. Canonical: 1 2 3 4 5.
        let s0: Vec<CommitRecord> = [0, 1, 3].map(|i| all[i].clone()).to_vec();
        let s1: Vec<CommitRecord> = [2, 4].map(|i| all[i].clone()).to_vec();
        let mut s = Strings::default();
        let (a0, p0, u0) = segment(&mut s, &s0);
        let (a1, p1, u1) = segment(&mut s, &s1);
        assert_ne!(
            s.paths.list, one_s.paths.list,
            "the fixture must reorder ids"
        );
        let ordered = [&u0[0], &u0[1], &u1[0], &u0[2], &u1[1]];
        let n = Renumber::new(&s, ordered.iter().map(|u| (u[0], &u[1..])));
        let parts = [
            Part {
                agg: &a0,
                postings: &p0,
                rank: &[0, 1, 3],
            },
            Part {
                agg: &a1,
                postings: &p1,
                rank: &[2, 4],
            },
        ];
        let a = aggregates(&parts, &n);
        let p = postings(&parts, &n);
        assert_eq!(n.strings.paths.list, one_s.paths.list);
        assert_eq!(n.strings.identities.list, one_s.identities.list);
        assert_eq!(a.author_day, one_a.author_day);
        assert_eq!(a.path_author, one_a.path_author);
        assert_eq!(p.by_path, one_p.by_path);
        assert_eq!(p.by_author, one_p.by_author);
        assert_eq!(p.edges, one_p.edges);
    }
}
