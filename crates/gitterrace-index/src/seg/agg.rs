//! The aggregates of a segment, built from records in canonical order: author x day and
//! path x author. Keyed by raw identity and raw path ids, never by an
//! option, so bots, mailmap, period and whitespace apply on read without a rebuild.
//! Merges are left out: no page counts them.

use std::collections::BTreeMap;

use crate::CommitRecord;
use crate::insights::fasthash::FastMap;
use crate::seg::bounds::{Cursor, MAX_BLOCK, put_varint, unzigzag, zigzag};
use crate::seg::format::section;
use crate::seg::{SegError, SegResult, Sink};

const DAY: i64 = 86_400;

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

    pub(crate) const fn merge(&mut self, o: &Self) {
        self.add += o.add;
        self.del += o.del;
        self.rewritten += o.rewritten;
        self.code_add += o.code_add;
        self.code_del += o.code_del;
        self.code_rewritten += o.code_rewritten;
    }
}

/// One author on one local day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AuthorDay {
    pub commits: u64,
    pub lines: Sums,
}

/// One path by one author over the whole segment (ownership).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PathAuthor {
    pub touches: u64,
    pub lines: Sums,
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

/// The three tables, sorted by key so writing them is deterministic.
#[derive(Debug, Default)]
pub(crate) struct Aggregates {
    /// (local day, identity).
    pub author_day: BTreeMap<(i64, u32), AuthorDay>,
    /// (path, identity).
    pub path_author: BTreeMap<(u32, u32), PathAuthor>,
}

impl Aggregates {
    /// Folds one record. Records must arrive in canonical order so ids are first-use ids.
    /// `who` and `paths` come from [`Strings::ids`].
    pub(crate) fn push(&mut self, r: &CommitRecord, who: u32, paths: &[u32]) {
        if r.merge {
            return;
        }
        let day = local_day(r);
        let ad = self.author_day.entry((day, who)).or_default();
        ad.commits += 1;
        // A commit touches a path once even if a rename lists it twice.
        let mut seen: Vec<u32> = Vec::with_capacity(paths.len());
        for (f, &p) in r.files.iter().zip(paths) {
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
            a.push(r, who, &paths);
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
    /// Writes sections author x day and path x author, sorted by key.
    pub(crate) fn encode(&self, sink: &mut Sink<'_>) -> SegResult<()> {
        pack(
            section::AUTHOR_DAY,
            self.author_day.iter(),
            |prev, (&(day, who), v), out| {
                let base = prev.map_or(0, |(&(d, _), _)| d);
                put_varint(out, zigzag(day.wrapping_sub(base)));
                put_varint(out, u64::from(who));
                put_varint(out, v.commits);
                put_sums(out, &v.lines);
            },
            sink,
        )?;
        pack(
            section::PATH_AUTHOR,
            self.path_author.iter(),
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
}

/// Rows of one author x day block, ascending by (day, identity).
pub(crate) fn decode_author_day(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(i64, u32, AuthorDay),
) -> SegResult<()> {
    let mut day = 0i64;
    for _ in 0..items {
        day = day.wrapping_add(unzigzag(c.varint()?));
        let who = c.varint()?;
        let who = u32_of(c, who)?;
        let commits = c.varint()?;
        f(
            day,
            who,
            AuthorDay {
                commits,
                lines: get_sums(c)?,
            },
        );
    }
    if c.is_empty() {
        Ok(())
    } else {
        Err(c.err("bytes after the last row"))
    }
}

/// Rows of one path x author block, ascending by (path, identity).
pub(crate) fn decode_path_author(
    c: &mut Cursor<'_>,
    items: u32,
    mut f: impl FnMut(u32, u32, PathAuthor),
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
        f(
            path,
            who,
            PathAuthor {
                touches,
                lines: get_sums(c)?,
            },
        );
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
mod tests {
    use std::collections::BTreeMap;

    use super::Aggregates;
    use crate::insights::fixtures::{chain, file, rec};
    use crate::insights::{Insights, Options, Period};

    fn fixture() -> Vec<crate::CommitRecord> {
        let mut v = chain();
        let mut renamed = file("z.rs", 2, 1, 30);
        renamed.old = Some("x.rs".into());
        let mut m = rec("e", "Cy", 50, vec![file("y.py", 9, 9, 9)]);
        m.merge = true;
        // y.py twice in one commit: still one touch.
        v.push(rec(
            "f",
            "Cy",
            45,
            vec![renamed, file("y.py", 4, 0, 3204), file("y.py", 1, 0, 3205)],
        ));
        v.push(m);
        v
    }

    /// Path x author summed over authors equals today's `file_churn` for every path.
    #[test]
    fn path_author_sums_equal_file_churn() {
        let recs = fixture();
        let (ids, agg) = Aggregates::build(&recs);
        let ins = Insights::new(&recs, Options::default(), None);
        let today = ins.file_churn(Period::All);
        let ws = Options::default().count_whitespace;
        let mut sums: BTreeMap<String, (usize, u64, u64)> = BTreeMap::new();
        for ((p, _), v) in &agg.path_author {
            let e = sums
                .entry(ids.paths.list[*p as usize].to_string())
                .or_default();
            e.0 += usize::try_from(v.touches).unwrap_or(0);
            let (a, d) = if ws {
                (v.lines.add, v.lines.del)
            } else {
                (v.lines.code_add, v.lines.code_del)
            };
            e.1 += a;
            e.2 += d;
        }
        sums.retain(|_, v| v.0 > 0);
        assert_eq!(sums.len(), today.len());
        for c in today {
            assert_eq!(
                sums[&c.path],
                (c.commits, c.additions, c.deletions),
                "{}",
                c.path
            );
        }
    }

    /// Author x day commits equal today's counted commits per author.
    #[test]
    fn author_day_counts_equal_counted_commits() {
        let recs = fixture();
        let (ids, agg) = Aggregates::build(&recs);
        let ins = Insights::new(&recs, Options::default(), None);
        let mut today: BTreeMap<String, u64> = BTreeMap::new();
        let src = ins.source();
        for c in ins.counted_rows(Period::All) {
            let (name, email) = src.ident(c.ident).unwrap_or_default();
            *today.entry(format!("{name}\u{0}{email}")).or_default() += 1;
        }
        let mut ours: BTreeMap<String, u64> = BTreeMap::new();
        for ((_, who), v) in &agg.author_day {
            *ours
                .entry(ids.identities.list[*who as usize].to_string())
                .or_default() += v.commits;
        }
        assert_eq!(ours, today);
    }

    #[test]
    fn path_author_and_ids_follow_first_use() {
        let (ids, agg) = Aggregates::build(&fixture());
        let names: Vec<&str> = ids.paths.list.iter().map(|s| &**s).collect();
        assert_eq!(names, ["x.rs", "y.py", "z.rs"]);
        let bob = ids.identities.get("Bob\u{0}bob@x.io");
        let y = ids.paths.get("y.py");
        let (Some(bob), Some(y)) = (bob, y) else {
            panic!("ids missing");
        };
        let row = agg.path_author[&(y, bob)];
        assert_eq!((row.touches, row.lines.add, row.lines.del), (2, 3300, 100));
        // The merge counts nowhere.
        assert!(ids.identities.get("Cy\u{0}cy@x.io").is_some());
        assert_eq!(agg.author_day.values().map(|v| v.commits).sum::<u64>(), 5);
    }

    /// Encoded blocks decode to the same rows, for tiny blocks too (the sink is the only
    /// writer, so this is the whole byte contract of sections 6 and 8).
    #[test]
    fn aggregates_round_trip_through_blocks() {
        use crate::seg::bounds::Cursor;
        let (_, agg) = Aggregates::build(&fixture());
        let mut blocks: Vec<(u16, u32, Vec<u8>)> = Vec::new();
        let r = agg.encode(&mut |s, n, b| {
            blocks.push((s, n, b.to_vec()));
            Ok(())
        });
        assert!(r.is_ok());
        let mut back = Aggregates::default();
        for (s, n, b) in &blocks {
            let mut c = Cursor::new(b, "t", 0);
            let r = if *s == crate::seg::format::section::AUTHOR_DAY {
                super::decode_author_day(&mut c, *n, |d, w, v| {
                    back.author_day.insert((d, w), v);
                })
            } else {
                super::decode_path_author(&mut c, *n, |p, w, v| {
                    back.path_author.insert((p, w), v);
                })
            };
            assert!(r.is_ok(), "{r:?}");
        }
        assert_eq!(back.author_day, agg.author_day);
        assert_eq!(back.path_author, agg.path_author);
        // Truncated block: an error, never a panic.
        let (s, n, b) = &blocks[0];
        let mut c = Cursor::new(&b[..b.len() - 1], "t", 0);
        assert!(
            super::decode_author_day(&mut c, *n, |_, _, _| {}).is_err(),
            "{s}"
        );
    }
}
