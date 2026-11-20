//! Path and author postings of a segment, with the rename edges that let a path log follow a
//! file back through its old names. Ordinals are segment-local
//! canonical positions. Path postings skip merges, as the follow table does today. Author
//! postings keep every commit, so a chip can still decide about merges on read.

use crate::CommitRecord;
use crate::seg::agg::{Packer, Strings, pack};
use crate::seg::bounds::{Cursor, put_varint};
use crate::seg::format::section;
use crate::seg::spill::Spill;
use crate::seg::{SegResult, Sink};

/// One row of a commit that touched a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Hit {
    pub ordinal: u32,
    pub row: u32,
}

/// The file at `row` of commit `ordinal` was called `old` before and `new` after.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Edge {
    pub ordinal: u32,
    pub row: u32,
    pub old: u32,
    pub new: u32,
}

#[derive(Debug, Default)]
pub(crate) struct Postings {
    /// Per path id, ascending.
    pub by_path: Vec<Vec<Hit>>,
    /// Per identity id, ascending ordinals.
    pub by_author: Vec<Vec<u32>>,
    /// Ascending by (ordinal, row).
    pub edges: Vec<Edge>,
    next: u32,
}

impl Postings {
    /// Tables already in canonical numbering (a merge result).
    pub(crate) fn from_parts(
        by_path: Vec<Vec<Hit>>,
        by_author: Vec<Vec<u32>>,
        edges: Vec<Edge>,
    ) -> Self {
        let next = by_author.iter().flatten().max().map_or(0, |m| m.saturating_add(1));
        Self { by_path, by_author, edges, next }
    }

    /// Folds the next record of the canonical order. `who` and `paths` come from
    /// [`Strings::ids`].
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the build spills path hits, the whole-table push serves merge tests"
        )
    )]
    pub(crate) fn push(&mut self, strings: &Strings, r: &CommitRecord, who: u32, paths: &[u32]) {
        self.fold(strings, r, who, paths, true);
    }

    /// As [`Postings::push`] with path hits left to a spill.
    pub(crate) fn push_without_paths(
        &mut self,
        strings: &Strings,
        r: &CommitRecord,
        who: u32,
        paths: &[u32],
    ) {
        self.fold(strings, r, who, paths, false);
    }

    fn fold(&mut self, strings: &Strings, r: &CommitRecord, who: u32, paths: &[u32], hits: bool) {
        let ord = self.next;
        self.next += 1;
        let who = who as usize;
        if self.by_author.len() <= who {
            self.by_author.resize_with(who + 1, Vec::new);
        }
        self.by_author[who].push(ord);
        for (row, f) in r.files.iter().enumerate() {
            let row = u32::try_from(row).unwrap_or(u32::MAX);
            let p = paths.get(row as usize).copied().unwrap_or(u32::MAX);
            if r.merge {
                continue;
            }
            if hits {
                if self.by_path.len() <= p as usize {
                    self.by_path.resize_with(p as usize + 1, Vec::new);
                }
                self.by_path[p as usize].push(Hit { ordinal: ord, row });
            }
            if let Some(old) = f.old.as_ref().and_then(|o| strings.paths.get(o)) {
                self.edges.push(Edge { ordinal: ord, row, old, new: p });
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn build<'a>(
        records: impl IntoIterator<Item = &'a CommitRecord>,
    ) -> (Strings, Self) {
        let mut s = Strings::default();
        let mut p = Self::default();
        for r in records {
            let (who, paths) = s.ids(r);
            p.push(&s, r, who, &paths);
        }
        (s, p)
    }

    /// Every hit that the follow rule files under `head`, newest first: the same answer as
    /// replaying all rows newest first with an alias table, but reading only the postings of
    /// names that can reach `head` through renames.
    // The test reference: path follow reads segments through the source.
    #[cfg(test)]
    pub(crate) fn follow(&self, head: u32) -> Vec<Hit> {
        follow(&self.by_path, &self.edges, head)
    }
}

/// See [`Postings::follow`]. `edges` ascending by (ordinal, row).
#[cfg(test)]
pub(crate) fn follow(by_path: &[Vec<Hit>], edges: &[Edge], head: u32) -> Vec<Hit> {
    // Names that could resolve to head: closure backwards over edges.
    let mut cand = std::collections::BTreeSet::from([head]);
    loop {
        let before = cand.len();
        for e in edges {
            if cand.contains(&e.new) {
                cand.insert(e.old);
            }
        }
        if cand.len() == before {
            break;
        }
    }
    // Replay newest first, rows ascending inside a commit: a row looks its name up, then its
    // rename (if any) sets the alias, exactly like today's follow. Only rows of candidate names
    // take part, so the work is the postings of those names.
    let mut ev: Vec<(u32, u32, Option<u32>, Option<&Edge>)> = cand
        .iter()
        .flat_map(|&p| {
            by_path
                .get(p as usize)
                .into_iter()
                .flatten()
                .map(move |h| (h.ordinal, h.row, Some(p), None))
        })
        .collect();
    ev.extend(
        edges.iter().filter(|e| cand.contains(&e.old)).map(|e| (e.ordinal, e.row, None, Some(e))),
    );
    // Lookup before edge on the same row: `None` sorts first, so put lookups at key 0.
    ev.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.cmp(&y.1)).then(x.3.is_some().cmp(&y.3.is_some())));
    let mut alias: std::collections::BTreeMap<u32, bool> = std::collections::BTreeMap::new();
    let resolves = |alias: &std::collections::BTreeMap<u32, bool>, p: u32| {
        alias.get(&p).copied().unwrap_or(p == head)
    };
    let mut out = Vec::new();
    for (ordinal, row, look, edge) in ev {
        if let Some(p) = look
            && resolves(&alias, p)
        {
            out.push(Hit { ordinal, row });
        }
        if let Some(e) = edge {
            let to = resolves(&alias, e.new);
            alias.insert(e.old, to);
        }
    }
    out
}

/// Most hits in one encoded item, so a long list splits into items under the block limit.
const CHUNK: usize = 16 * 1024;

impl Postings {
    /// Writes sections path postings, rename edges and author postings. An item is
    /// (id delta, count, entries), a list longer than [`CHUNK`] spans several items of the
    /// same id (delta 0). Empty lists are left out.
    // The build encodes spilled postings, whole tables serve the tests.
    #[cfg(test)]
    pub(crate) fn encode(&self, sink: &mut Sink<'_>) -> SegResult<()> {
        let paths = chunks(&self.by_path);
        pack(
            section::PATH_POSTINGS,
            paths.iter().copied(),
            |prev, (id, hits), out| path_item(prev.map(|p| p.0), id, hits, out),
            sink,
        )?;
        self.encode_rest(sink)
    }

    /// Sections rename edges and author postings, for a writer that streamed the path
    /// postings through [`encode_spilled`].
    pub(crate) fn encode_rest(&self, sink: &mut Sink<'_>) -> SegResult<()> {
        pack(
            section::RENAMES,
            self.edges.iter().copied(),
            |prev, e, out| {
                put_varint(out, u64::from(e.ordinal - prev.map_or(0, |p| p.ordinal)));
                put_varint(out, u64::from(e.row));
                put_varint(out, u64::from(e.old));
                put_varint(out, u64::from(e.new));
            },
            sink,
        )?;
        let authors = chunks(&self.by_author);
        pack(
            section::AUTHOR_POSTINGS,
            authors.iter().copied(),
            |prev, (id, ords), out| {
                put_varint(out, u64::from(id - prev.map_or(0, |p| p.0)));
                put_varint(out, ords.len() as u64);
                let mut last = 0u32;
                for &o in ords {
                    put_varint(out, u64::from(o.wrapping_sub(last)));
                    last = o;
                }
            },
            sink,
        )
    }
}

fn path_item(prev: Option<u32>, id: u32, hits: &[Hit], out: &mut Vec<u8>) {
    put_varint(out, u64::from(id - prev.unwrap_or(0)));
    put_varint(out, hits.len() as u64);
    let mut last = 0u32;
    for h in hits {
        put_varint(out, u64::from(h.ordinal.wrapping_sub(last)));
        put_varint(out, u64::from(h.row));
        last = h.ordinal;
    }
}

/// Section path postings from a spill of (path, ordinal, row) entries, the same bytes as
/// [`Postings::encode`] writes for the same hits. Memory is one chunk.
pub(crate) fn encode_spilled(spill: Spill, sink: &mut Sink<'_>) -> SegResult<()> {
    let mut p = Packer::new(section::PATH_POSTINGS, sink);
    let mut prev: Option<u32> = None;
    let mut cur: Option<u32> = None;
    let mut hits: Vec<Hit> = Vec::new();
    let mut item = Vec::new();
    let mut emit = |p: &mut Packer<'_>, prev: &mut Option<u32>, id: u32, hits: &[Hit]| {
        item.clear();
        path_item(if p.fresh() { None } else { *prev }, id, hits, &mut item);
        if !p.push(&item)? {
            item.clear();
            path_item(None, id, hits, &mut item);
            p.push(&item)?;
        }
        *prev = Some(id);
        SegResult::Ok(())
    };
    spill.finish(|(path, ordinal, row)| {
        if cur.is_some_and(|c| c != path) || hits.len() == CHUNK {
            if let Some(c) = cur {
                emit(&mut p, &mut prev, c, &hits).map_err(std::io::Error::other)?;
            }
            hits.clear();
        }
        cur = Some(path);
        hits.push(Hit { ordinal, row });
        Ok(())
    })?;
    if let Some(c) = cur {
        emit(&mut p, &mut prev, c, &hits)?;
    }
    p.flush()
}

fn chunks<T>(lists: &[Vec<T>]) -> Vec<(u32, &[T])> {
    let mut out = Vec::new();
    for (id, v) in lists.iter().enumerate() {
        let id = u32::try_from(id).unwrap_or(u32::MAX);
        out.extend(v.chunks(CHUNK).map(|c| (id, c)));
    }
    out
}

fn id_of(c: &Cursor<'_>, base: u32, delta: u64) -> SegResult<u32> {
    u32::try_from(delta).ok().and_then(|d| base.checked_add(d)).ok_or_else(|| c.err("id over u32"))
}

/// One path postings block: `f(path, hits)` per item, ids ascending.
pub(crate) fn decode_path_postings(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(u32, &[Hit]),
) -> SegResult<()> {
    let mut id = 0;
    let mut hits = Vec::new();
    for _ in 0..items {
        let d = c.varint()?;
        id = id_of(c, id, d)?;
        let n = c.count(CHUNK)?;
        hits.clear();
        let mut ord = 0u32;
        for _ in 0..n {
            let d = c.varint()?;
            ord = id_of(c, ord, d)?;
            let row = c.varint()?;
            hits.push(Hit { ordinal: ord, row: id_of(c, 0, row)? });
        }
        f(id, &hits);
    }
    end(c)
}

/// One rename edges block, ascending by (ordinal, row).
pub(crate) fn decode_renames(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(Edge),
) -> SegResult<()> {
    let mut ord = 0u32;
    for _ in 0..items {
        let d = c.varint()?;
        ord = id_of(c, ord, d)?;
        let (row, old, new) = (c.varint()?, c.varint()?, c.varint()?);
        f(Edge {
            ordinal: ord,
            row: id_of(c, 0, row)?,
            old: id_of(c, 0, old)?,
            new: id_of(c, 0, new)?,
        });
    }
    end(c)
}

/// One author postings block: `f(identity, ordinals)` per item.
pub(crate) fn decode_author_postings(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(u32, &[u32]),
) -> SegResult<()> {
    let mut id = 0;
    let mut ords = Vec::new();
    for _ in 0..items {
        let d = c.varint()?;
        id = id_of(c, id, d)?;
        let n = c.count(CHUNK)?;
        ords.clear();
        let mut ord = 0u32;
        for _ in 0..n {
            let d = c.varint()?;
            ord = id_of(c, ord, d)?;
            ords.push(ord);
        }
        f(id, &ords);
    }
    end(c)
}

fn end(c: &Cursor<'_>) -> SegResult<()> {
    if c.is_empty() { Ok(()) } else { Err(c.err("bytes after the last item")) }
}

#[cfg(test)]
mod tests {
    use super::{Hit, Postings};
    use crate::insights::fixtures::{file, rec};
    use crate::insights::{Insights, Options};

    fn renamed(new: &str, old: &str) -> crate::FileStat {
        let mut f = file(new, 1, 1, 5);
        f.old = Some(old.into());
        f
    }

    /// Renames back and forth, a name reused for another file, a swap in one commit.
    fn fixture() -> Vec<crate::CommitRecord> {
        let mut m = rec("m", "Bob", 6, vec![file("a", 1, 0, 1)]);
        m.merge = true;
        vec![
            rec("1", "Ann", 0, vec![file("a", 1, 0, 1), file("b", 1, 0, 1)]),
            rec("2", "Ann", 1, vec![renamed("c", "a")]),
            rec("3", "Bob", 2, vec![file("a", 2, 0, 2), file("c", 1, 0, 2)]),
            rec("4", "Bob", 3, vec![renamed("a", "b"), renamed("b", "a")]),
            rec("5", "Ann", 4, vec![renamed("d", "c"), file("a", 1, 1, 2)]),
            m,
            rec("6", "Cy", 7, vec![renamed("e", "d")]),
        ]
    }

    #[test]
    fn follow_matches_todays_follow_table() {
        let recs = fixture();
        let ins = Insights::new(&recs, Options::default(), None);
        let (ids, p) = Postings::build(&recs);
        let mut checked = 0;
        for (id, name) in ids.paths.list.iter().enumerate() {
            let want: Vec<Hit> = ins
                .follow_hits(name)
                .into_iter()
                .map(|(ordinal, row)| Hit { ordinal, row })
                .collect();
            if want.is_empty() {
                continue;
            }
            assert_eq!(p.follow(u32::try_from(id).unwrap_or(u32::MAX)), want, "{name}");
            checked += 1;
        }
        // e (a renamed twice) and the swapped pair: at least two heads with history.
        assert!(checked >= 2, "{checked}");
    }

    #[test]
    fn author_postings_keep_merges() {
        let (ids, p) = Postings::build(&fixture());
        let bob = ids.identities.get("Bob\u{0}bob@x.io");
        assert_eq!(bob.map(|b| p.by_author[b as usize].clone()), Some(vec![2, 3, 5]));
    }

    #[test]
    fn postings_round_trip_through_blocks() {
        use crate::seg::bounds::Cursor;
        use crate::seg::format::section;
        let (_, p) = Postings::build(&fixture());
        let mut blocks: Vec<(u16, u32, Vec<u8>)> = Vec::new();
        assert!(
            p.encode(&mut |s, n, b| {
                blocks.push((s, n, b.to_vec()));
                Ok(())
            })
            .is_ok()
        );
        let mut by_path = vec![Vec::new(); p.by_path.len()];
        let mut by_author = vec![Vec::new(); p.by_author.len()];
        let mut edges = Vec::new();
        for (s, n, b) in &blocks {
            let mut c = Cursor::new(b, "t", 0);
            let r = match *s {
                section::PATH_POSTINGS => super::decode_path_postings(&mut c, *n, |id, h| {
                    by_path[id as usize].extend_from_slice(h);
                }),
                section::RENAMES => super::decode_renames(&mut c, *n, |e| edges.push(e)),
                _ => super::decode_author_postings(&mut c, *n, |id, o| {
                    by_author[id as usize].extend_from_slice(o);
                }),
            };
            assert!(r.is_ok(), "{r:?}");
        }
        assert_eq!(by_path, p.by_path);
        assert_eq!(by_author, p.by_author);
        assert_eq!(edges, p.edges);
    }

    /// The spilled writer gives the in-memory writer's bytes, spilled or not.
    #[test]
    fn spilled_path_postings_equal_in_memory_bytes() {
        use crate::seg::spill::Spill;
        let (_, p) = Postings::build(&fixture());
        let collect = |f: &mut dyn FnMut(&mut crate::seg::Sink<'_>)| {
            let mut out: Vec<(u16, u32, Vec<u8>)> = Vec::new();
            f(&mut |s, n, b| {
                if s == crate::seg::format::section::PATH_POSTINGS {
                    out.push((s, n, b.to_vec()));
                }
                Ok(())
            });
            out
        };
        let want = collect(&mut |sink| assert!(p.encode(sink).is_ok()));
        for limit in [1, 3, usize::MAX] {
            let got = collect(&mut |sink| {
                let mut sp = Spill::new(limit, tempfile::tempfile);
                for (id, hits) in p.by_path.iter().enumerate().rev() {
                    for h in hits {
                        assert!(sp.push((id as u32, h.ordinal, h.row)).is_ok());
                    }
                }
                assert!(super::encode_spilled(sp, sink).is_ok());
            });
            assert_eq!(got, want, "limit {limit}");
        }
    }
}
