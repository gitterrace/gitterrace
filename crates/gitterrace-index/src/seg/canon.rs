//! Lives of files: the stable key of the rename-aware aggregates (sections 7, 9 and 10).
//!
//! A life starts when a name that holds none is touched: an add, a copy (an add without
//! `-C`) or a re-add after a delete. A rename moves the life of its old name to the new one, a
//! delete ends it. Lives are numbered by first use while non-merge commits are replayed in
//! order-file order, rows of a commit looked up against the state before the commit, so a swap
//! `a <-> b` keeps both lives. An id once written is never renumbered: an append continues from
//! the newest section 7. When two branches rename one name, the rename first in that order
//! keeps the life and the other starts a new one at its rename, as a copy does.
//!
//! Section 7 is fixed 4 B items, whole in every segment (the newest wins): one per path id,
//! the last life the name had with `HOLDS` set while it still holds it, then one per life,
//! its last name with `HOLDS` set while that name still holds it (the life is live).

use crate::CommitRecord;
#[cfg(any(test, feature = "oracle"))]
use crate::seg::agg::add_count;
use crate::seg::agg::pack;
use crate::seg::bounds::{Cursor, MAX_BLOCK};
use crate::seg::format::section;
use crate::seg::{SegError, SegResult, Sink};

/// Top bit of a section 7 item: the name holds the life now, or the life is live.
pub(crate) const HOLDS: u32 = 1 << 31;
/// A path id that never held a life (touched only as a rename source or by merges).
pub(crate) const NONE: u32 = u32::MAX;

/// The life state after the commits replayed so far.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Lives {
    /// Per path id: last life, `HOLDS` while held, `NONE` if never.
    by_name: Vec<u32>,
    name: Vec<u32>,
}

impl Lives {
    /// Life of every row of one record (`paths` from `Strings::ids`, `olds` the old-name ids),
    /// then the state after it. Merges change nothing and get no lives.
    pub(crate) fn push(
        &mut self,
        r: &CommitRecord,
        paths: &[u32],
        olds: &[Option<u32>],
    ) -> Vec<u32> {
        if r.merge {
            return Vec::new();
        }
        let need = paths.iter().chain(olds.iter().flatten()).max().map_or(0, |&m| m as usize + 1);
        if self.by_name.len() < need {
            self.by_name.resize(need, NONE);
        }
        let held = |v: u32| (v != NONE && v & HOLDS != 0).then_some(v & !HOLDS);
        let mut out = Vec::with_capacity(paths.len());
        let mut claimed: Vec<u32> = Vec::new();
        for (row, &p) in paths.iter().enumerate() {
            let from = olds.get(row).copied().flatten().unwrap_or(p);
            let life = match held(self.by_name[from as usize]) {
                Some(l) if !claimed.contains(&l) => l,
                // Same name twice in one commit: one touch, one life.
                Some(l) if from == p && out.contains(&l) => l,
                _ => self.start(p),
            };
            claimed.push(life);
            out.push(life);
        }
        // Releases first, so a swap or a rename onto a freed name ends held.
        for (row, f) in r.files.iter().enumerate() {
            if let Some(Some(o)) = olds.get(row) {
                self.release(*o);
            }
            if f.deleted {
                self.release(paths[row]);
            }
        }
        for (row, f) in r.files.iter().enumerate() {
            if !f.deleted {
                let (p, life) = (paths[row], out[row]);
                self.by_name[p as usize] = life | HOLDS;
                self.name[life as usize] = p;
            }
        }
        out
    }

    fn start(&mut self, p: u32) -> u32 {
        let life = u32::try_from(self.name.len()).unwrap_or(HOLDS - 1).min(HOLDS - 1);
        self.name.push(p);
        life
    }

    fn release(&mut self, p: u32) {
        if let Some(v) = self.by_name.get_mut(p as usize)
            && *v != NONE
        {
            *v &= !HOLDS;
        }
    }

    #[cfg(test)]
    pub(crate) const fn lives(&self) -> usize {
        self.name.len()
    }

    /// Section 7 for a set of `paths` path ids.
    pub(crate) fn encode(&self, paths: usize, sink: &mut Sink<'_>) -> SegResult<()> {
        let by_name = (0..paths).map(|p| self.by_name.get(p).copied().unwrap_or(NONE));
        let lives = self.name.iter().enumerate().map(|(life, &n)| {
            let held = self.by_name.get(n as usize).copied().unwrap_or(NONE);
            let live = held != NONE && held & HOLDS != 0 && (held & !HOLDS) as usize == life;
            n | if live { HOLDS } else { 0 }
        });
        let items: Vec<u32> = by_name.chain(lives).collect();
        if items.is_empty() {
            return Ok(());
        }
        pack(section::CANON, items, |_, v, out| out.extend_from_slice(&v.to_le_bytes()), sink)
    }

    /// The state section 7 holds, for an append to continue. `paths` is the set's path count
    /// when it was written. Read whole: an append is a write, allowed its peak.
    pub(crate) fn decode(blocks: &[&[u8]], paths: usize, file: &str) -> SegResult<Self> {
        let mut all = Vec::new();
        for b in blocks {
            if b.len() % 4 != 0 || b.len() > MAX_BLOCK {
                return Err(SegError::unreadable(file, 0, "canon block length"));
            }
            let mut c = Cursor::new(b, file, 0);
            while !c.is_empty() {
                all.push(c.u32()?);
            }
        }
        if all.len() < paths {
            return Err(SegError::unreadable(file, 0, "canon shorter than the paths"));
        }
        let name: Vec<u32> = all[paths..].iter().map(|v| v & !HOLDS).collect();
        all.truncate(paths);
        for &v in &all {
            if v != NONE && (v & !HOLDS) as usize >= name.len() {
                return Err(SegError::unreadable(file, 0, "canon life out of range"));
            }
        }
        if name.iter().any(|&n| n as usize >= paths) {
            return Err(SegError::unreadable(file, 0, "canon name out of range"));
        }
        Ok(Self { by_name: all, name })
    }
}

/// Section 7 and the section 10 touches per life re-derived from records in order-file order.
///
/// One replay: the oracle an append is checked against (the shape of [`dump`]).
#[cfg(any(test, feature = "oracle"))]
pub fn canon_of(records: &[CommitRecord]) -> (Vec<u32>, Vec<u64>) {
    let mut s = crate::seg::agg::Strings::default();
    let mut agg = crate::seg::agg::Aggregates::default();
    for r in records {
        let (who, paths) = s.ids(r);
        let olds = olds(&s, r);
        agg.push(r, who, &paths, &olds);
    }
    let mut canon = Vec::new();
    let r = agg.lives.encode(s.paths.list.len(), &mut |_, _, b| {
        canon.extend(b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)));
        Ok(())
    });
    debug_assert!(r.is_ok());
    let mut touches = vec![0u64; agg.lives.name.len()];
    for (&(life, _), v) in &agg.life_author {
        touches[life as usize] += v.touches;
    }
    (canon, touches)
}

/// Old-name ids of the rows of `r`, already interned.
pub(crate) fn olds(s: &crate::seg::agg::Strings, r: &CommitRecord) -> Vec<Option<u32>> {
    r.files.iter().map(|f| f.old.as_deref().and_then(|o| s.paths.get(o))).collect()
}

/// What a store's lives hold, for guards: the newest section 7 and, per life, the touches
/// of section 10 summed over every segment. Reads the store whole, a test hook. Not an API.
#[cfg(any(test, feature = "oracle"))]
pub fn dump(store: &crate::seg::read::Store) -> SegResult<(Vec<u32>, Vec<u64>)> {
    let mut canon = Vec::new();
    let mut buf = Vec::new();
    if let Some(seg) = store.segments.last() {
        for e in seg.blocks(section::CANON) {
            let b = seg.read_block(e, &mut buf)?;
            canon.extend(b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)));
        }
    }
    let mut touches: Vec<u64> = Vec::new();
    for seg in &store.segments {
        for e in seg.blocks(section::CANON_PATH_AUTHOR) {
            let b = seg.read_block(e, &mut buf)?;
            let mut c = Cursor::new(b, seg.name(), e.offset);
            crate::seg::agg::decode_path_author(&mut c, e.items, |life, _, v| {
                let l = life as usize;
                if touches.len() <= l {
                    touches.resize(l + 1, 0);
                }
                add_count(&mut touches[l], v.touches)
            })?;
        }
    }
    Ok((canon, touches))
}

/// Line sums as a tuple: raw add, del, rewritten, then the whitespace-blind three.
pub type SumsTuple = [u64; 6];

/// The rows of sections 9 and 10 and the merges of section 6, summed over a set's segments.
///
/// Months: touches and sums added, last touch the newest. The shape both [`life_tables`] and
/// [`life_tables_of`] give, so the equivalence test compares a set with its records.
#[cfg(any(test, feature = "oracle"))]
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LifeTables {
    /// (life, local month) to (touches, sums, last touch).
    pub months: std::collections::BTreeMap<(u32, i64), (u64, SumsTuple, i64)>,
    /// (life, identity) to (touches, sums).
    pub authors: std::collections::BTreeMap<(u32, u32), (u64, SumsTuple)>,
    /// (local day, identity) to merges, merge-free days left out.
    pub merges: std::collections::BTreeMap<(i64, u32), u64>,
}

#[cfg(any(test, feature = "oracle"))]
const fn tuple(s: &crate::seg::agg::Sums) -> SumsTuple {
    [s.add, s.del, s.rewritten, s.code_add, s.code_del, s.code_rewritten]
}

/// `a += b` per field, `None` past u64 (`a` may be part added then: the caller drops it).
#[cfg(any(test, feature = "oracle"))]
#[must_use]
fn add_tuple(a: &mut SumsTuple, b: &SumsTuple) -> Option<()> {
    for (x, y) in a.iter_mut().zip(b) {
        add_count(x, *y)?;
    }
    Some(())
}

#[cfg(any(test, feature = "oracle"))]
impl LifeTables {
    /// The row summed in, `None` when a sum passes u64.
    fn month(&mut self, life: u32, month: i64, v: &crate::seg::agg::LifeMonth) -> Option<()> {
        let e = self.months.entry((life, month)).or_insert((0, [0; 6], i64::MIN));
        add_count(&mut e.0, v.touches)?;
        add_tuple(&mut e.1, &tuple(&v.lines))?;
        e.2 = e.2.max(v.last);
        Some(())
    }

    /// The row summed in, `None` when a sum passes u64.
    fn author(&mut self, life: u32, who: u32, v: &crate::seg::agg::PathAuthor) -> Option<()> {
        let e = self.authors.entry((life, who)).or_insert((0, [0; 6]));
        add_count(&mut e.0, v.touches)?;
        add_tuple(&mut e.1, &tuple(&v.lines))
    }

    /// The merges summed in, `None` when the sum passes u64.
    fn merge(&mut self, day: i64, who: u32, merges: u64) -> Option<()> {
        if merges > 0 {
            add_count(self.merges.entry((day, who)).or_default(), merges)?;
        }
        Some(())
    }
}

/// [`LifeTables`] as a store holds them. Reads the store whole, a test hook. Not an API.
#[cfg(any(test, feature = "oracle"))]
pub fn life_tables(store: &crate::seg::read::Store) -> SegResult<LifeTables> {
    let mut t = LifeTables::default();
    let mut buf = Vec::new();
    for seg in &store.segments {
        for e in seg.blocks(section::PATH_TOUCH) {
            let b = seg.read_block(e, &mut buf)?;
            let mut c = Cursor::new(b, seg.name(), e.offset);
            crate::seg::agg::decode_life_month(&mut c, e.items, |l, m, v| t.month(l, m, &v))?;
        }
        for e in seg.blocks(section::CANON_PATH_AUTHOR) {
            let b = seg.read_block(e, &mut buf)?;
            let mut c = Cursor::new(b, seg.name(), e.offset);
            crate::seg::agg::decode_path_author(&mut c, e.items, |l, w, v| t.author(l, w, &v))?;
        }
        for e in seg.blocks(section::AUTHOR_DAY) {
            let b = seg.read_block(e, &mut buf)?;
            let mut c = Cursor::new(b, seg.name(), e.offset);
            crate::seg::agg::decode_author_day(&mut c, e.items, |d, w, v| t.merge(d, w, v.merges))?;
        }
    }
    Ok(t)
}

/// [`LifeTables`] re-derived from records in order-file order, in one replay: the oracle.
#[cfg(any(test, feature = "oracle"))]
pub fn life_tables_of(records: &[CommitRecord]) -> LifeTables {
    let mut s = crate::seg::agg::Strings::default();
    let mut agg = crate::seg::agg::Aggregates::default();
    for r in records {
        let (who, paths) = s.ids(r);
        let olds = olds(&s, r);
        agg.push(r, who, &paths, &olds);
    }
    // Counts from records: u32 per row, so no sum passes u64 and every add is kept.
    let mut t = LifeTables::default();
    let mut ok = true;
    for (&(l, m), v) in &agg.life_month {
        ok &= t.month(l, m, v).is_some();
    }
    for (&(l, w), v) in &agg.life_author {
        ok &= t.author(l, w, v).is_some();
    }
    for (&(d, w), v) in &agg.author_day {
        ok &= t.merge(d, w, v.merges).is_some();
    }
    debug_assert!(ok, "record counts summed past u64");
    t
}

#[cfg(test)]
mod tests {
    use super::{HOLDS, Lives, NONE, canon_of};
    use crate::insights::fixtures::{file, rec};
    use crate::seg::agg::Strings;

    fn renamed(new: &str, old: &str) -> crate::FileStat {
        let mut f = file(new, 1, 1, 5);
        f.old = Some(old.into());
        f
    }

    fn gone(p: &str) -> crate::FileStat {
        let mut f = file(p, 0, 3, 0);
        f.deleted = true;
        f
    }

    fn lives_of(recs: &[crate::CommitRecord]) -> (Strings, Vec<Vec<u32>>, Lives) {
        let mut s = Strings::default();
        let mut l = Lives::default();
        let mut out = Vec::new();
        for r in recs {
            let (_, paths) = s.ids(r);
            let o = super::olds(&s, r);
            out.push(l.push(r, &paths, &o));
        }
        (s, out, l)
    }

    #[test]
    fn a_chain_keeps_one_life_and_a_reuse_starts_another() {
        let recs = vec![
            rec("1", "Ann", 0, vec![file("a", 3, 0, 3), file("x", 1, 0, 1)]),
            rec("2", "Ann", 1, vec![renamed("b", "a")]),
            rec("3", "Bob", 2, vec![renamed("c", "b"), gone("x")]),
            rec("4", "Bob", 3, vec![file("x", 1, 0, 1), file("d", 3, 0, 3)]),
        ];
        let (ids, rows, state) = lives_of(&recs);
        let (s, l) = (&ids, &state);
        assert_eq!(rows, [vec![0, 1], vec![0], vec![0, 1], vec![2, 3]]);
        let tip = s.paths.get("c").unwrap_or(NONE);
        assert_eq!(l.name[0], tip);
        let (canon, _) = canon_of(&recs);
        assert_eq!(canon.len(), s.paths.list.len() + l.lives());
        let x = s.paths.get("x").unwrap_or(NONE) as usize;
        assert_eq!(canon[x], 2 | HOLDS, "x holds its new life");
        let p = s.paths.list.len();
        assert_eq!(canon[p + 1] & HOLDS, 0, "the deleted x life is dead");
        assert_ne!(canon[p] & HOLDS, 0, "the chain is live");
    }

    #[test]
    fn a_swap_keeps_both_lives() {
        let recs = vec![
            rec("1", "Ann", 0, vec![file("a", 1, 0, 1), file("b", 1, 0, 1)]),
            rec("2", "Ann", 1, vec![renamed("a", "b"), renamed("b", "a")]),
        ];
        let (_, rows, l) = lives_of(&recs);
        assert_eq!(rows[1], [1, 0]);
        assert_eq!(l.lives(), 2);
    }

    /// Side renames x to y first in the order, main renames x to z: y keeps the life.
    #[test]
    fn the_first_rename_of_a_fork_keeps_the_life() {
        let recs = vec![
            rec("1", "Ann", 0, vec![file("x", 3, 0, 3)]),
            rec("2", "Ann", 1, vec![renamed("y", "x")]),
            rec("3", "Bob", 2, vec![renamed("z", "x")]),
        ];
        let (_, rows, _) = lives_of(&recs);
        assert_eq!(rows, [vec![0], vec![0], vec![1]]);
    }

    #[test]
    fn section_seven_round_trips() {
        let recs = vec![
            rec("1", "Ann", 0, vec![file("a", 1, 0, 1)]),
            rec("2", "Ann", 1, vec![renamed("b", "a"), gone("q")]),
        ];
        let (s, _, l) = lives_of(&recs);
        let mut blocks = Vec::new();
        let written = l.encode(s.paths.list.len(), &mut |_, _, b| {
            blocks.push(b.to_vec());
            Ok(())
        });
        assert!(written.is_ok());
        let slices: Vec<&[u8]> = blocks.iter().map(Vec::as_slice).collect();
        let decoded = Lives::decode(&slices, s.paths.list.len(), "t");
        assert_eq!(decoded.ok().map(|d| d.name), Some(l.name));
        assert!(Lives::decode(&slices, s.paths.list.len() + 9, "t").is_err());
    }
}
