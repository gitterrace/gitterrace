//! Opens one container and reads its blocks.
//!
//! Open costs two bounded preads (footer and toc), a block is read only when asked for, into a
//! caller buffer, and its crc is checked before any byte of it is handed out.

use std::fs::File;
use std::path::Path;

use crate::seg::bounds::MAX_TOC;
use crate::seg::format::{
    FOOTER_LEN, Footer, HEADER_LEN, Kind, StringBase, TOC_ENTRY_LEN, TocEntry, check_header,
    decode_toc,
};
use crate::seg::io::{open_named, read_exact_at};
use crate::seg::manifest::{FileEntry, Manifest, check_name};
use crate::seg::{Rank, SegError, SegResult};

#[cfg(test)]
thread_local! {
    /// Blocks read from disk on this thread, for the guards that bound re-reads.
    pub(crate) static BLOCK_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// An open container. Holds its own `File` (Windows `seek_read` moves the cursor).
#[derive(Debug)]
pub struct Container {
    name: String,
    file: File,
    footer: Footer,
    toc: Vec<TocEntry>,
}

impl Container {
    pub fn open(dir: &Path, name: &str, kind: Kind) -> SegResult<Self> {
        check_name(name)?;
        let unreadable = |off, what| SegError::unreadable(name, off, what);
        let file = open_named(dir, name).map_err(|_| unreadable(0, "cannot open"))?;
        let len = file.metadata()?.len();
        if len < HEADER_LEN + FOOTER_LEN {
            return Err(unreadable(0, "shorter than header and footer"));
        }
        let mut head = [0; 16];
        read_exact_at(&file, &mut head, 0).map_err(|_| unreadable(0, "header read"))?;
        check_header(&head, kind, name)?;
        let mut fb = [0; 64];
        let at = len - FOOTER_LEN;
        read_exact_at(&file, &mut fb, at).map_err(|_| unreadable(at, "footer read"))?;
        let footer = Footer::decode(&fb, len, kind, name)?;
        // decode bounded toc_count by MAX_TOC, so this buffer is at most 96 KiB.
        let toc_len = footer.toc_count as usize * TOC_ENTRY_LEN as usize;
        debug_assert!(toc_len <= MAX_TOC * TOC_ENTRY_LEN as usize);
        let mut tb = vec![0; toc_len];
        read_exact_at(&file, &mut tb, footer.toc_offset)
            .map_err(|_| unreadable(footer.toc_offset, "toc read"))?;
        let toc = decode_toc(&tb, &footer, name)?;
        Ok(Self {
            name: name.to_owned(),
            file,
            footer,
            toc,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn base(&self) -> StringBase {
        self.footer.base
    }

    pub const fn items(&self) -> u32 {
        self.footer.items
    }

    /// The blocks of one section, in file order. Empty for a reserved or absent section.
    pub fn blocks(&self, section: u16) -> &[TocEntry] {
        let start = self.toc.partition_point(|e| e.section < section);
        let end = self.toc.partition_point(|e| e.section <= section);
        &self.toc[start..end]
    }

    pub fn toc(&self) -> &[TocEntry] {
        &self.toc
    }

    /// Reads one block into `buf` (reused across calls) and checks its crc.
    pub fn read_block<'b>(&self, e: &TocEntry, buf: &'b mut Vec<u8>) -> SegResult<&'b [u8]> {
        // The toc check bounded len by MAX_BLOCK and the range by the file.
        #[cfg(test)]
        BLOCK_READS.with(|n| n.set(n.get() + 1));
        buf.clear();
        buf.resize(e.len as usize, 0);
        read_exact_at(&self.file, buf, e.offset)
            .map_err(|_| SegError::unreadable(&self.name, e.offset, "block read"))?;
        if crc32fast::hash(buf) != e.crc {
            return Err(SegError::unreadable(&self.name, e.offset, "block crc"));
        }
        Ok(buf)
    }

    /// Reads every block, checking every crc. For the hostile and unreadable guards.
    pub fn check_all(&self) -> SegResult<()> {
        let mut buf = Vec::new();
        for e in &self.toc {
            self.read_block(e, &mut buf)?;
        }
        Ok(())
    }

    /// Reads one fixed-width record by index across a section's blocks. The whole block is
    /// read and its crc checked first, so a flipped byte is `Unreadable`, never a wrong value.
    /// A block is at most `MAX_BLOCK`, so this costs one bounded pread.
    pub fn record_at(
        &self,
        section: u16,
        index: u32,
        width: usize,
        out: &mut [u8],
    ) -> SegResult<()> {
        let mut first = 0u32;
        for e in self.blocks(section) {
            if u64::from(e.items) * width as u64 != u64::from(e.len) {
                return Err(SegError::unreadable(
                    &self.name,
                    e.offset,
                    "fixed-width block length",
                ));
            }
            if index < first.saturating_add(e.items) {
                let at = (index - first) as usize * width;
                let out = out
                    .get_mut(..width)
                    .ok_or_else(|| SegError::unreadable(&self.name, e.offset, "width"))?;
                let mut buf = Vec::new();
                let block = self.read_block(e, &mut buf)?;
                let rec = block
                    .get(at..at + width)
                    .ok_or_else(|| SegError::unreadable(&self.name, e.offset, "record read"))?;
                out.copy_from_slice(rec);
                return Ok(());
            }
            first = first.saturating_add(e.items);
        }
        Err(SegError::unreadable(
            &self.name,
            0,
            "record index past the section",
        ))
    }

    /// The first `out.len()` bytes of block `e`, by pread, crc not checked. For routing
    /// tables built at open only: every use compares the value with the block once that
    /// block is read checked (`DiskSource::rank_of`).
    pub(crate) fn head_unchecked(&self, e: &TocEntry, out: &mut [u8]) -> SegResult<()> {
        if out.len() as u64 > u64::from(e.len) {
            return Err(SegError::unreadable(&self.name, e.offset, "block head"));
        }
        read_exact_at(&self.file, out, e.offset)
            .map_err(|_| SegError::unreadable(&self.name, e.offset, "block head read"))
    }

    /// The canonical rank of a segment-local ordinal (section `RANK`), crc checked.
    pub fn rank(&self, ordinal: crate::seg::Ordinal) -> SegResult<Rank> {
        let mut b = [0; 4];
        self.record_at(crate::seg::format::section::RANK, ordinal.0, 4, &mut b)?;
        Ok(Rank(u32::from_le_bytes(b)))
    }

    /// Ranks of many ordinals, in the order given. Reads each rank block it needs once (crc
    /// checked), instead of one pread per ordinal: a path's postings name hundreds of
    /// thousands on a large history.
    pub fn ranks(&self, ordinals: &[u32]) -> SegResult<Vec<Rank>> {
        let blocks = self.blocks(crate::seg::format::section::RANK);
        let mut starts = Vec::with_capacity(blocks.len());
        let mut first = 0u32;
        for e in blocks {
            if u64::from(e.items) * 4 != u64::from(e.len) {
                return Err(SegError::unreadable(
                    &self.name,
                    e.offset,
                    "fixed-width block length",
                ));
            }
            starts.push(first);
            first = first.saturating_add(e.items);
        }
        let mut loaded: Option<(usize, Vec<u8>)> = None;
        let mut out = Vec::with_capacity(ordinals.len());
        for &o in ordinals {
            let i = starts.partition_point(|&s| s <= o).checked_sub(1);
            let Some(i) = i.filter(|&i| o < starts[i].saturating_add(blocks[i].items)) else {
                return Err(SegError::unreadable(
                    &self.name,
                    0,
                    "record index past the section",
                ));
            };
            if loaded.as_ref().is_none_or(|(at, _)| *at != i) {
                let mut buf = Vec::new();
                self.read_block(&blocks[i], &mut buf)?;
                loaded = Some((i, buf));
            }
            let Some((_, buf)) = loaded.as_ref() else {
                return Err(SegError::unreadable(&self.name, 0, "rank block"));
            };
            let at = (o - starts[i]) as usize * 4;
            let b: [u8; 4] = buf
                .get(at..at + 4)
                .and_then(|s| s.try_into().ok())
                .ok_or_else(|| SegError::unreadable(&self.name, blocks[i].offset, "rank"))?;
            out.push(Rank(u32::from_le_bytes(b)));
        }
        Ok(out)
    }
}

/// The live set of a cache: manifest checked, every container's footer checked, and the string
/// bases consistent in manifest order, so string ids cannot shift.
#[derive(Debug)]
pub struct Store {
    pub manifest: Manifest,
    pub segments: Vec<Container>,
    pub strings: Vec<Container>,
    pub order: Option<Container>,
    /// The message column's files in manifest order, empty when it is off.
    pub messages: Vec<Container>,
    /// The stored first-parent chain, absent until the update after a build writes it.
    pub chain: Option<Container>,
    /// The tip touch table (`crate::tip`), absent until the update after a build writes it.
    pub tip: Option<Container>,
}

impl Store {
    /// Opens the cache in `dir`. `Ok(None)` when there is no manifest. Any inconsistency is
    /// `Unreadable`, which the caller maps to delete and rebuild.
    pub fn open(dir: &Path) -> SegResult<Option<Self>> {
        let Some(manifest) = Manifest::load(dir)? else {
            return Ok(None);
        };
        Self::of(dir, manifest).map(Some)
    }

    /// Opens the set `manifest` names, which need not be the published one: a merge reads its
    /// staged set this way to write the chain before the manifest naming both goes live.
    pub(crate) fn of(dir: &Path, manifest: Manifest) -> SegResult<Self> {
        manifest.verify(dir)?;
        let (mut segments, mut strings, mut order) = (Vec::new(), Vec::new(), None);
        let (mut messages, mut chain, mut tip) = (Vec::new(), None, None);
        for f in &manifest.live {
            match Kind::from_u32(u32::from(f.kind)) {
                Some(Kind::Segment) => segments.push(Container::open(dir, &f.name, Kind::Segment)?),
                Some(Kind::Strings) => strings.push(Container::open(dir, &f.name, Kind::Strings)?),
                Some(Kind::Order) if order.is_none() => {
                    order = Some(Container::open(dir, &f.name, Kind::Order)?);
                }
                Some(Kind::Order) => {
                    return Err(SegError::unreadable(&f.name, 0, "second order file"));
                }
                Some(Kind::Messages) => {
                    messages.push(Container::open(dir, &f.name, Kind::Messages)?);
                }
                Some(Kind::Chain) if chain.is_none() => {
                    chain = Some(Container::open(dir, &f.name, Kind::Chain)?);
                }
                Some(Kind::Chain) => {
                    return Err(SegError::unreadable(&f.name, 0, "second chain file"));
                }
                Some(Kind::Tip) if tip.is_none() => {
                    tip = Some(Container::open(dir, &f.name, Kind::Tip)?);
                }
                Some(Kind::Tip) => {
                    return Err(SegError::unreadable(&f.name, 0, "second tip file"));
                }
                None => {}
            }
        }
        check_bases(&segments, &strings)?;
        if let Some(o) = &order
            && o.items() != manifest.commits
        {
            return Err(SegError::unreadable(
                o.name(),
                0,
                "order count differs from manifest",
            ));
        }
        Ok(Self {
            manifest,
            segments,
            strings,
            order,
            messages,
            chain,
            tip,
        })
    }

    /// Reads every block of every live container (forces every crc and bound check).
    pub fn check_all(&self) -> SegResult<()> {
        self.segments
            .iter()
            .chain(&self.strings)
            .chain(&self.order)
            .chain(&self.messages)
            .chain(&self.chain)
            .chain(&self.tip)
            .try_for_each(Container::check_all)
    }

    pub fn entries(&self) -> &[FileEntry] {
        &self.manifest.live
    }
}

/// Delta `n` was written against the cumulative counts of deltas `0..n`. A delta's own counts are
/// its section item totals. Each segment was written with its delta, so its base is one of the
/// delta bases, in order. A retired segment leaves its delta live (later ids depend on it), so
/// there may be more deltas than segments, never fewer.
fn check_bases(segments: &[Container], strings: &[Container]) -> SegResult<()> {
    use crate::seg::format::section::{IDENTITIES, LABELS, PATHS};
    let mut bases = Vec::with_capacity(strings.len());
    let mut sum = StringBase::default();
    let total = |c: &Container, s| {
        c.blocks(s)
            .iter()
            .try_fold(0u32, |a, e| a.checked_add(e.items))
    };
    for st in strings {
        if st.base() != sum {
            return Err(SegError::unreadable(
                st.name(),
                0,
                "string base differs from manifest order",
            ));
        }
        bases.push(sum);
        let add = |a: u32, b: Option<u32>| b.and_then(|b| a.checked_add(b));
        let (Some(paths), Some(identities), Some(labels)) = (
            add(sum.paths, total(st, PATHS)),
            add(sum.identities, total(st, IDENTITIES)),
            add(sum.labels, total(st, LABELS)),
        ) else {
            return Err(SegError::unreadable(st.name(), 0, "string count overflow"));
        };
        sum = StringBase {
            paths,
            identities,
            labels,
        };
    }
    let mut next = 0;
    for seg in segments {
        let at = bases[next..].iter().position(|&b| b == seg.base());
        let at =
            at.ok_or_else(|| SegError::unreadable(seg.name(), 0, "segment string base unknown"))?;
        next += at + 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::seg::format::section;
    use crate::seg::manifest::Manifest;
    use crate::seg::strings::{StringDelta, write_delta};
    use crate::seg::write::{ContainerWriter, write_order};
    use crate::seg::{Ordinal, fingerprint};

    /// A two-segment store: segment 0 with two paths, segment 1 with one more.
    fn build(dir: &Path) -> SegResult<Manifest> {
        let mut live = Vec::new();
        let mut base = StringBase::default();
        for (g, paths) in [(0u32, vec!["a", "b"]), (1, vec!["c"])] {
            let mut w = ContainerWriter::new(dir, &format!("seg-1-{g}.gts"), Kind::Segment, base)?;
            w.block(section::COMMITS, 2, &[g as u8; 40])?;
            let ranks: Vec<u8> = [g * 2, g * 2 + 1]
                .iter()
                .flat_map(|r| r.to_le_bytes())
                .collect();
            w.block(section::RANK, 2, &ranks)?;
            live.push(w.finish()?);
            let delta = StringDelta {
                paths: paths.into_iter().map(String::from).collect(),
                ..Default::default()
            };
            let (entry, next) = write_delta(dir, &format!("str-1-{g}.gts"), base, &delta)?;
            live.push(entry);
            base = next;
        }
        let oids: Vec<[u8; 20]> = (0..4u8).map(|i| [i; 20]).collect();
        live.push(write_order(dir, "ord-1.gts", &oids)?);
        let mut m = Manifest {
            generation: 1,
            fingerprint: fingerprint(&oids),
            commits: 4,
            shallow: 0,
            live,
            retired: vec![],
        };
        m.commit(dir)?;
        Ok(m)
    }

    #[test]
    fn round_trip_and_rank_by_pread() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        build(dir.path())?;
        let s = Store::open(dir.path())?.ok_or_else(|| SegError::BadName("none".into()))?;
        s.check_all()?;
        assert_eq!(s.segments[1].rank(Ordinal(1))?, Rank(3));
        assert!(s.segments[1].rank(Ordinal(2)).is_err());
        let mut buf = Vec::new();
        let e = s.segments[0].blocks(section::COMMITS)[0];
        assert_eq!(s.segments[0].read_block(&e, &mut buf)?, &[0u8; 40]);
        assert!(s.segments[0].blocks(section::CANON).is_empty());
        Ok(())
    }

    #[test]
    fn every_cut_and_flip_of_a_segment_is_unreadable() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        build(dir.path())?;
        let path = dir.path().join("seg-1-0.gts");
        let good = fs::read(&path)?;
        let open_all = |d: &Path| Store::open(d).and_then(|s| s.map_or(Ok(()), |s| s.check_all()));
        for cut in [0, 1, good.len() / 2, good.len() - 1] {
            fs::write(&path, &good[..cut])?;
            assert!(
                open_all(dir.path()).is_err_and(|e| e.is_unreadable()),
                "cut {cut}"
            );
        }
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] ^= 0x40;
            fs::write(&path, &b)?;
            // The container alone, so the manifest's footer crc does not mask the inner checks.
            let r = Container::open(dir.path(), "seg-1-0.gts", Kind::Segment)
                .and_then(|c| c.check_all());
            assert!(r.is_err_and(|e| e.is_unreadable()), "flip {i}");
            assert!(open_all(dir.path()).is_err(), "store flip {i}");
        }
        Ok(())
    }

    #[test]
    fn string_base_mismatch_is_unreadable() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        let mut m = build(dir.path())?;
        // Swap the two deltas: the bases then do not accumulate in manifest order.
        m.live.swap(1, 3);
        m.commit(dir.path())?;
        assert!(Store::open(dir.path()).is_err_and(|e| e.is_unreadable()));
        Ok(())
    }

    #[test]
    fn retired_segment_keeps_its_delta() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        let mut m = build(dir.path())?;
        let seg0 = m.live.remove(0);
        m.retired.push(seg0.name);
        m.commit(dir.path())?;
        let s = Store::open(dir.path())?.ok_or_else(|| SegError::BadName("none".into()))?;
        assert_eq!((s.segments.len(), s.strings.len()), (1, 2));
        Ok(())
    }

    #[test]
    fn missing_manifest_is_none_and_missing_file_unreadable() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        assert!(Store::open(dir.path())?.is_none());
        build(dir.path())?;
        fs::remove_file(dir.path().join("str-1-1.gts"))?;
        assert!(Store::open(dir.path()).is_err_and(|e| e.is_unreadable()));
        Ok(())
    }
}
