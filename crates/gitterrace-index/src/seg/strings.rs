//! String deltas: the strings each segment introduced, ids cumulative in manifest order.
//!
//! Per segment, the paths, identities and labels it introduced. Ids are cumulative over the deltas
//! in manifest order. Items are u16 length plus UTF-8, blocks are cut at 1 MiB, and a lookup reads
//! the one block holding the id.

use std::collections::HashMap;
use std::path::Path;

use crate::seg::bounds::{Cursor, MAX_BLOCK, MAX_STRING};
use crate::seg::format::{Kind, StringBase, section};
use crate::seg::manifest::FileEntry;
use crate::seg::read::Container;
use crate::seg::write::ContainerWriter;
use crate::seg::{IdentId, LabelId, PathId, SegError, SegResult};

/// The strings one segment introduced, each list in first-use order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StringDelta {
    pub paths: Vec<String>,
    pub identities: Vec<String>,
    pub labels: Vec<String>,
}

/// Assigns ids by first use during a build, on top of the strings earlier segments wrote.
///
/// Holds only the new strings plus a map of them: earlier ids are looked up by the caller
/// through `known` (the build already has the previous tables or rebuilds whole).
#[derive(Debug, Default)]
pub struct Interner {
    base: StringBase,
    delta: StringDelta,
    paths: HashMap<String, u32>,
    identities: HashMap<String, u32>,
    labels: HashMap<String, u32>,
}

fn intern(
    map: &mut HashMap<String, u32>,
    list: &mut Vec<String>,
    base: u32,
    kind: &'static str,
    s: &str,
) -> SegResult<u32> {
    if s.len() > MAX_STRING {
        return Err(SegError::TooLong { kind, len: s.len(), max: MAX_STRING });
    }
    if let Some(&id) = map.get(s) {
        return Ok(id);
    }
    let id = u32::try_from(list.len())
        .ok()
        .and_then(|n| base.checked_add(n))
        .ok_or(SegError::TooLong { kind, len: list.len(), max: u32::MAX as usize })?;
    map.insert(s.to_owned(), id);
    list.push(s.to_owned());
    Ok(id)
}

impl Interner {
    pub fn new(base: StringBase) -> Self {
        Self { base, ..Self::default() }
    }

    pub fn path(&mut self, s: &str) -> SegResult<PathId> {
        intern(&mut self.paths, &mut self.delta.paths, self.base.paths, "path", s).map(PathId)
    }

    pub fn identity(&mut self, s: &str) -> SegResult<IdentId> {
        let base = self.base.identities;
        intern(&mut self.identities, &mut self.delta.identities, base, "identity", s).map(IdentId)
    }

    pub fn label(&mut self, s: &str) -> SegResult<LabelId> {
        intern(&mut self.labels, &mut self.delta.labels, self.base.labels, "label", s).map(LabelId)
    }

    pub const fn base(&self) -> StringBase {
        self.base
    }

    pub fn into_delta(self) -> StringDelta {
        self.delta
    }
}

/// Writes a delta against `base`. Returns its manifest entry and the next base.
pub fn write_delta(
    dir: &Path,
    name: &str,
    base: StringBase,
    delta: &StringDelta,
) -> SegResult<(FileEntry, StringBase)> {
    let mut w = ContainerWriter::new(dir, name, Kind::Strings, base)?;
    let mut next = base;
    for (sec, list, slot) in [
        (section::PATHS, &delta.paths, &mut next.paths),
        (section::IDENTITIES, &delta.identities, &mut next.identities),
        (section::LABELS, &delta.labels, &mut next.labels),
    ] {
        write_list(&mut w, sec, list)?;
        let n = u32::try_from(list.len()).ok().and_then(|n| slot.checked_add(n));
        *slot = n.ok_or(SegError::TooLong {
            kind: "string count",
            len: list.len(),
            max: u32::MAX as usize,
        })?;
    }
    Ok((w.finish()?, next))
}

fn write_list(w: &mut ContainerWriter, sec: u16, list: &[String]) -> SegResult<()> {
    let mut block = Vec::new();
    let mut items = 0u32;
    for s in list {
        let len = u16::try_from(s.len()).map_err(|_| SegError::TooLong {
            kind: "string",
            len: s.len(),
            max: MAX_STRING,
        })?;
        if block.len() + 2 + s.len() > MAX_BLOCK {
            w.block(sec, items, &block)?;
            block.clear();
            items = 0;
        }
        block.extend_from_slice(&len.to_le_bytes());
        block.extend_from_slice(s.as_bytes());
        items += 1;
    }
    if items > 0 {
        w.block(sec, items, &block)?;
    }
    Ok(())
}

/// Looks up strings by cumulative id across the live deltas. Reads one block per lookup into
/// a reused buffer, so resident memory is the toc of each delta plus one block.
pub struct Strings<'a> {
    deltas: &'a [Container],
    buf: Vec<u8>,
}

impl<'a> Strings<'a> {
    pub const fn new(deltas: &'a [Container]) -> Self {
        Self { deltas, buf: Vec::new() }
    }

    pub fn path(&mut self, id: PathId) -> SegResult<String> {
        self.get(section::PATHS, id.0, |b| b.paths)
    }

    pub fn identity(&mut self, id: IdentId) -> SegResult<String> {
        self.get(section::IDENTITIES, id.0, |b| b.identities)
    }

    pub fn label(&mut self, id: LabelId) -> SegResult<String> {
        self.get(section::LABELS, id.0, |b| b.labels)
    }

    fn get(&mut self, sec: u16, id: u32, base: fn(StringBase) -> u32) -> SegResult<String> {
        // Deltas are in manifest order with increasing bases: the last one at or below id.
        let at = self.deltas.partition_point(|d| base(d.base()) <= id);
        let d = at.checked_sub(1).and_then(|i| self.deltas.get(i));
        let d =
            d.ok_or_else(|| SegError::unreadable("strings", 0, "string id below every delta"))?;
        let mut first = base(d.base());
        for e in d.blocks(sec) {
            let end = first.saturating_add(e.items);
            if id < end {
                let block = d.read_block(e, &mut self.buf)?;
                let mut c = Cursor::new(block, d.name(), e.offset);
                for i in first..=id {
                    let n = usize::from(c.u16()?);
                    let s = c.bytes(n)?;
                    if i == id {
                        let s = std::str::from_utf8(s).map_err(|_| c.err("string not utf-8"))?;
                        return Ok(s.to_owned());
                    }
                }
            }
            first = end;
        }
        Err(SegError::unreadable(d.name(), 0, "string id past its delta"))
    }

    /// Parses every string of every delta, for the guards.
    pub fn check_all(&mut self) -> SegResult<()> {
        for d in self.deltas {
            for e in d.toc() {
                let block = d.read_block(e, &mut self.buf)?;
                let mut c = Cursor::new(block, d.name(), e.offset);
                for _ in 0..e.items {
                    let n = usize::from(c.u16()?);
                    std::str::from_utf8(c.bytes(n)?).map_err(|_| c.err("string not utf-8"))?;
                }
                if !c.is_empty() {
                    return Err(c.err("trailing bytes in string block"));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_cumulative_and_round_trip() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        let mut i = Interner::new(StringBase::default());
        assert_eq!(i.path("a")?, PathId(0));
        assert_eq!(i.path("b")?, PathId(1));
        assert_eq!(i.path("a")?, PathId(0));
        i.identity("Ann <a@x>")?;
        let (_, base) = write_delta(dir.path(), "str-1-0.gts", i.base(), &i.into_delta())?;
        assert_eq!(base, StringBase { paths: 2, identities: 1, labels: 0 });
        let mut j = Interner::new(base);
        assert_eq!(j.path("c")?, PathId(2));
        assert_eq!(j.label("feat")?, LabelId(0));
        write_delta(dir.path(), "str-1-1.gts", base, &j.into_delta())?;
        let ds = [
            Container::open(dir.path(), "str-1-0.gts", Kind::Strings)?,
            Container::open(dir.path(), "str-1-1.gts", Kind::Strings)?,
        ];
        let mut s = Strings::new(&ds);
        s.check_all()?;
        assert_eq!(s.path(PathId(1))?, "b");
        assert_eq!(s.path(PathId(2))?, "c");
        assert_eq!(s.identity(IdentId(0))?, "Ann <a@x>");
        assert_eq!(s.label(LabelId(0))?, "feat");
        assert!(s.path(PathId(3)).is_err_and(|e| e.is_unreadable()));
        assert!(s.identity(IdentId(1)).is_err());
        Ok(())
    }

    #[test]
    fn overlong_string_errors_without_truncation() {
        let mut i = Interner::new(StringBase::default());
        let long = "x".repeat(MAX_STRING + 1);
        assert!(matches!(i.path(&long), Err(SegError::TooLong { .. })));
        assert!(matches!(i.identity(&long), Err(SegError::TooLong { .. })));
        assert!(i.path(&"x".repeat(MAX_STRING)).is_ok());
        assert_eq!(i.into_delta().identities.len(), 0);
    }

    #[test]
    fn many_strings_span_blocks() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        let big = "y".repeat(60_000);
        let delta = StringDelta {
            paths: (0..40).map(|i| format!("{big}{i}")).collect(),
            ..Default::default()
        };
        write_delta(dir.path(), "str-1-0.gts", StringBase::default(), &delta)?;
        let ds = [Container::open(dir.path(), "str-1-0.gts", Kind::Strings)?];
        assert!(ds[0].blocks(section::PATHS).len() > 1);
        let mut s = Strings::new(&ds);
        assert_eq!(s.path(PathId(39))?, format!("{big}39"));
        Ok(())
    }
}
