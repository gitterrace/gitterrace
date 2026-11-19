//! The aggregates of a segment, built from records in canonical order: author x day,
//! path x author, and the rename-aware tables keyed by life (see `canon`): life x local month
//! and life x author. Keyed by raw identity, raw path and life ids, never by an option, so
//! bots, mailmap, period and whitespace apply on read without a rebuild. Merges count only in
//! `AuthorDay::merges`: their rows repeat the branch's lines.

use std::collections::BTreeMap;

use gitterrace_git::DAY;
use rustc_hash::FxHashMap as FastMap;

use crate::CommitRecord;
use crate::seg::bounds::{Cursor, MAX_BLOCK, put_varint, unzigzag, zigzag};
use crate::seg::canon::Lives;
use crate::seg::format::section;
use crate::seg::{SegError, SegResult, Sink};

/// Line sums, raw and with whitespace-only changes ignored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Sums {
    pub add: u64,
    pub del: u64,
    pub rewritten: u64,
    pub code_add: u64,
    pub code_del: u64,
    pub code_rewritten: u64,
}

impl Sums {
    fn add_file(&mut self, f: &crate::FileStat) {
        let c = f.code_stat();
        self.add += u64::from(f.additions);
        self.del += u64::from(f.deletions);
        self.rewritten += u64::from(f.rewritten);
        self.code_add += u64::from(c.additions);
        self.code_del += u64::from(c.deletions);
        self.code_rewritten += u64::from(c.rewritten);
    }

    /// Adds `o`, or `None` when a sum passes u64 (stored counts only get there). `self` is
    /// unchanged on `None`.
    #[must_use]
    pub(crate) const fn merge(&mut self, o: &Self) -> Option<()> {
        let (Some(add), Some(del), Some(rewritten)) = (
            self.add.checked_add(o.add),
            self.del.checked_add(o.del),
            self.rewritten.checked_add(o.rewritten),
        ) else {
            return None;
        };
        let (Some(code_add), Some(code_del), Some(code_rewritten)) = (
            self.code_add.checked_add(o.code_add),
            self.code_del.checked_add(o.code_del),
            self.code_rewritten.checked_add(o.code_rewritten),
        ) else {
            return None;
        };
        *self = Self {
            add,
            del,
            rewritten,
            code_add,
            code_del,
            code_rewritten,
        };
        Some(())
    }
}

/// What an unreadable error names when stored counts summed past u64. A build never gets
/// there (a commit adds u32 counts), so only a hostile or broken file does.
pub(crate) const SUM_OVER: &str = "counts sum past u64";

/// `a += b` on a stored counter, `None` past u64.
#[must_use]
pub(crate) const fn add_count(a: &mut u64, b: u64) -> Option<()> {
    match a.checked_add(b) {
        Some(v) => {
            *a = v;
            Some(())
        }
        None => None,
    }
}

/// One author on one local day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AuthorDay {
    /// Non-merge commits.
    pub commits: u64,
    pub merges: u64,
    pub lines: Sums,
}

/// One path by one author over the whole segment (ownership).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PathAuthor {
    pub touches: u64,
    pub lines: Sums,
}

/// One life in one local month (section 9). Whole-life numbers are the sum over months.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LifeMonth {
    pub touches: u64,
    pub lines: Sums,
    /// Newest author time of a touch.
    pub last: i64,
}

/// First-use string ids, in the order the canonical stream introduces them.
#[derive(Debug, Default)]
pub(crate) struct Interner {
    ids: FastMap<Box<str>, u32>,
    pub list: Vec<Box<str>>,
}

impl Interner {
    pub(crate) fn id(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.ids.get(s) {
            return i;
        }
        let i = u32::try_from(self.list.len()).unwrap_or(u32::MAX);
        self.ids.insert(s.into(), i);
        self.list.push(s.into());
        i
    }

    pub(crate) fn get(&self, s: &str) -> Option<u32> {
        self.ids.get(s).copied()
    }
}

/// String ids a segment sees: the deltas before it (ids are global until a full merge)
/// plus what it introduces. Identity is `name\0email`, as written in the commit.
#[derive(Debug, Default)]
pub(crate) struct Strings {
    pub identities: Interner,
    pub paths: Interner,
}

impl Strings {
    /// Ids of one record: its identity and the path id of each row. Old names of renames
    /// are interned before the new one, in row order.
    pub(crate) fn ids(&mut self, r: &CommitRecord) -> (u32, Vec<u32>) {
        let who = self.identities.id(&identity(r));
        let paths = r
            .files
            .iter()
            .map(|f| {
                let p = self.paths.id(&f.path);
                if let Some(old) = &f.old {
                    self.paths.id(old);
                }
                p
            })
            .collect();
        (who, paths)
    }
}

/// The tables, sorted by key so writing them is deterministic.
#[derive(Debug, Default)]
pub(crate) struct Aggregates {
    /// (local day, identity).
    pub author_day: BTreeMap<(i64, u32), AuthorDay>,
    /// (path, identity).
    pub path_author: BTreeMap<(u32, u32), PathAuthor>,
    /// (life, local month as year * 12 + month - 1).
    pub life_month: BTreeMap<(u32, i64), LifeMonth>,
    /// (life, identity).
    pub life_author: BTreeMap<(u32, u32), PathAuthor>,
    /// The life state, continued from the set's newest section 7 by an append.
    pub lives: Lives,
}

impl Aggregates {
    /// Folds one record. Records must arrive in canonical order so ids are first-use ids.
    /// `who` and `paths` come from [`Strings::ids`].
    /// `olds` are the old-name ids of renamed rows.
    pub(crate) fn push(&mut self, r: &CommitRecord, who: u32, paths: &[u32], olds: &[Option<u32>]) {
        let day = local_day(r);
        let ad = self.author_day.entry((day, who)).or_default();
        if r.merge {
            ad.merges += 1;
            return;
        }
        ad.commits += 1;
        let lives = self.lives.push(r, paths, olds);
        let month = local_month(day);
        // A commit touches a path, and a life, once even if a rename lists it twice.
        let mut seen: Vec<u32> = Vec::with_capacity(paths.len());
        let mut seen_life: Vec<u32> = Vec::with_capacity(paths.len());
        for ((f, &p), &l) in r.files.iter().zip(paths).zip(&lives) {
            ad.lines.add_file(f);
            let first = !seen.contains(&p);
            if first {
                seen.push(p);
            }
            let pa = self.path_author.entry((p, who)).or_default();
            pa.lines.add_file(f);
            if first {
                pa.touches += 1;
            }
            let first = !seen_life.contains(&l);
            if first {
                seen_life.push(l);
            }
            let lm = self.life_month.entry((l, month)).or_default();
            let la = self.life_author.entry((l, who)).or_default();
            lm.lines.add_file(f);
            la.lines.add_file(f);
            if first {
                lm.touches += 1;
                la.touches += 1;
                lm.last = if lm.touches == 1 {
                    r.time
                } else {
                    lm.last.max(r.time)
                };
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn build<'a>(
        records: impl IntoIterator<Item = &'a CommitRecord>,
    ) -> (Strings, Self) {
        let mut s = Strings::default();
        let mut a = Self::default();
        for r in records {
            let (who, paths) = s.ids(r);
            let olds = crate::seg::canon::olds(&s, r);
            a.push(r, who, &paths, &olds);
        }
        (s, a)
    }
}

pub(crate) fn identity(r: &CommitRecord) -> String {
    format!("{}\u{0}{}", r.author, r.email)
}

/// Days since 1970-01-01 in the author's own time zone, as `insights::local_day`.
pub(crate) fn local_day(r: &CommitRecord) -> i64 {
    r.time.saturating_add(i64::from(r.offset)).div_euclid(DAY)
}

/// The local month of a local day: year * 12 + month - 1.
pub(crate) fn local_month(day: i64) -> i64 {
    let (y, m, _) = gitterrace_git::civil_from_days(day);
    y * 12 + m - 1
}

/// Groups encoded items into blocks of at most [`MAX_BLOCK`] bytes. Every block decodes on
/// its own (deltas restart at a block), so a reader can seek to any block.
pub(crate) struct Packer<'w> {
    section: u16,
    buf: Vec<u8>,
    items: u32,
    sink: &'w mut Sink<'w>,
}

impl<'w> Packer<'w> {
    pub(crate) fn new(section: u16, sink: &'w mut Sink<'w>) -> Self {
        Self {
            section,
            buf: Vec::new(),
            items: 0,
            sink,
        }
    }

    /// True when the next item starts a block, so its deltas must be absolute.
    pub(crate) const fn fresh(&self) -> bool {
        self.items == 0
    }

    /// Adds one item. The caller encodes with [`Packer::fresh`] in mind and re-encodes when
    /// this returns the item back because the block was cut first.
    pub(crate) fn push(&mut self, item: &[u8]) -> SegResult<bool> {
        if item.len() > MAX_BLOCK {
            return Err(SegError::TooLong {
                kind: "item",
                len: item.len(),
                max: MAX_BLOCK,
            });
        }
        if self.buf.len() + item.len() > MAX_BLOCK {
            self.flush()?;
            return Ok(false);
        }
        self.buf.extend_from_slice(item);
        self.items += 1;
        Ok(true)
    }

    pub(crate) fn flush(&mut self) -> SegResult<()> {
        if self.items > 0 {
            (self.sink)(self.section, self.items, &self.buf)?;
            self.buf.clear();
            self.items = 0;
        }
        Ok(())
    }
}

/// Encodes items through `enc(prev, out)`, where `prev` is `None` at a block start.
pub(crate) fn pack<T: Copy>(
    section: u16,
    items: impl IntoIterator<Item = T>,
    mut enc: impl FnMut(Option<T>, T, &mut Vec<u8>),
    sink: &mut Sink<'_>,
) -> SegResult<()> {
    let mut p = Packer::new(section, sink);
    let mut prev = None;
    let mut item = Vec::new();
    for x in items {
        item.clear();
        enc(if p.fresh() { None } else { prev }, x, &mut item);
        if !p.push(&item)? {
            item.clear();
            enc(None, x, &mut item);
            p.push(&item)?;
        }
        prev = Some(x);
    }
    p.flush()
}

fn put_sums(out: &mut Vec<u8>, s: &Sums) {
    for v in [s.add, s.del, s.rewritten] {
        put_varint(out, v);
    }
    // Code counts differ rarely: store the difference, nearly always three zero bytes.
    put_varint(out, s.add.wrapping_sub(s.code_add));
    put_varint(out, s.del.wrapping_sub(s.code_del));
    put_varint(out, s.rewritten.wrapping_sub(s.code_rewritten));
}

fn get_sums(c: &mut Cursor<'_>) -> SegResult<Sums> {
    let (add, del, rewritten) = (c.varint()?, c.varint()?, c.varint()?);
    Ok(Sums {
        add,
        del,
        rewritten,
        code_add: add.wrapping_sub(c.varint()?),
        code_del: del.wrapping_sub(c.varint()?),
        code_rewritten: rewritten.wrapping_sub(c.varint()?),
    })
}

fn u32_of(c: &Cursor<'_>, v: u64) -> SegResult<u32> {
    u32::try_from(v).map_err(|_| c.err("id over u32"))
}

impl Aggregates {
    /// Writes sections 6 to 10, sorted by key. `paths` is the set's path count after this
    /// segment, the length of the per-name part of section 7.
    pub(crate) fn encode(&self, paths: usize, sink: &mut Sink<'_>) -> SegResult<()> {
        pack(
            section::AUTHOR_DAY,
            self.author_day.iter(),
            |prev, (&(day, who), v), out| {
                let base = prev.map_or(0, |(&(d, _), _)| d);
                put_varint(out, zigzag(day.wrapping_sub(base)));
                put_varint(out, u64::from(who));
                put_varint(out, v.commits);
                put_varint(out, v.merges);
                put_sums(out, &v.lines);
            },
            sink,
        )?;
        self.lives.encode(paths, sink)?;
        pack_path_author(section::PATH_AUTHOR, &self.path_author, sink)?;
        pack(
            section::PATH_TOUCH,
            self.life_month.iter(),
            |prev, (&(life, month), v), out| {
                let base = prev.map_or(0, |(&(l, _), _)| l);
                put_varint(out, u64::from(life.wrapping_sub(base)));
                put_varint(out, zigzag(month));
                put_varint(out, v.touches);
                put_varint(out, zigzag(v.last));
                put_sums(out, &v.lines);
            },
            sink,
        )?;
        pack_path_author(section::CANON_PATH_AUTHOR, &self.life_author, sink)
    }
}

/// Section 8 or 10: (key, identity) rows, key delta-coded.
fn pack_path_author(
    sec: u16,
    rows: &BTreeMap<(u32, u32), PathAuthor>,
    sink: &mut Sink<'_>,
) -> SegResult<()> {
    pack(
        sec,
        rows.iter(),
        |prev, (&(path, who), v), out| {
            let base = prev.map_or(0, |(&(p, _), _)| p);
            put_varint(out, u64::from(path.wrapping_sub(base)));
            put_varint(out, u64::from(who));
            put_varint(out, v.touches);
            put_sums(out, &v.lines);
        },
        sink,
    )
}

/// Rows of one author x day block, ascending by (day, identity).
pub(crate) fn decode_author_day(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(i64, u32, AuthorDay) -> Option<()>,
) -> SegResult<()> {
    let mut day = 0i64;
    for _ in 0..items {
        day = day.wrapping_add(unzigzag(c.varint()?));
        let who = c.varint()?;
        let who = u32_of(c, who)?;
        let commits = c.varint()?;
        let merges = c.varint()?;
        let row = f(
            day,
            who,
            AuthorDay {
                commits,
                merges,
                lines: get_sums(c)?,
            },
        );
        row.ok_or_else(|| c.err(SUM_OVER))?;
    }
    if c.is_empty() {
        Ok(())
    } else {
        Err(c.err("bytes after the last row"))
    }
}

/// Rows of one life x month block (section 9), ascending by (life, month).
#[cfg(any(test, feature = "oracle"))]
pub(crate) fn decode_life_month(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(u32, i64, LifeMonth) -> Option<()>,
) -> SegResult<()> {
    let mut life = 0u32;
    for _ in 0..items {
        let d = c.varint()?;
        life = life
            .checked_add(u32_of(c, d)?)
            .ok_or_else(|| c.err("life id over u32"))?;
        let month = unzigzag(c.varint()?);
        let touches = c.varint()?;
        let last = unzigzag(c.varint()?);
        let row = f(
            life,
            month,
            LifeMonth {
                touches,
                lines: get_sums(c)?,
                last,
            },
        );
        row.ok_or_else(|| c.err(SUM_OVER))?;
    }
    if c.is_empty() {
        Ok(())
    } else {
        Err(c.err("bytes after the last row"))
    }
}

/// Rows of one path x author block (section 8, or 10 keyed by life), ascending by key.
pub(crate) fn decode_path_author(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(u32, u32, PathAuthor) -> Option<()>,
) -> SegResult<()> {
    let mut path = 0u32;
    for _ in 0..items {
        let d = c.varint()?;
        path = path
            .checked_add(u32_of(c, d)?)
            .ok_or_else(|| c.err("path id over u32"))?;
        let who = c.varint()?;
        let who = u32_of(c, who)?;
        let touches = c.varint()?;
        let row = f(
            path,
            who,
            PathAuthor {
                touches,
                lines: get_sums(c)?,
            },
        );
        row.ok_or_else(|| c.err(SUM_OVER))?;
    }
    if c.is_empty() {
        Ok(())
    } else {
        Err(c.err("bytes after the last row"))
    }
}

#[cfg(test)]
#[expect(
    clippy::many_single_char_names,
    reason = "short names for the segment parts of a fixture"
)]
mod tests;
