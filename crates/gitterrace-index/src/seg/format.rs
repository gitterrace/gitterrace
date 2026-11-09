//! The container layout: header, blocks, table of contents, footer. See docs/architecture.md.
//!
//! A 16 B header, blocks, a table of contents and a 64 B footer. Encoding and the checks of a
//! parsed footer and toc live here, file access lives in `read` and `write`.

use crate::seg::bounds::{Cursor, MAX_BLOCK, MAX_TOC};
use crate::seg::{SegError, SegResult};

pub(crate) const MAGIC: [u8; 4] = *b"GTS1";
pub(crate) const FOOTER_MAGIC: [u8; 4] = *b"GTSF";
/// The on-disk format, shared with `VERSION`.
pub(crate) const VERSION: u32 = 7;
pub(crate) const HEADER_LEN: u64 = 16;
pub(crate) const FOOTER_LEN: u64 = 64;
pub(crate) const TOC_ENTRY_LEN: u64 = 24;

/// What a container holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Segment = 1,
    Strings = 2,
    Order = 3,
    /// The message column of commit search (`messages.rs`).
    Messages = 4,
    /// The first-parent chain of the tip and its line totals (`chain.rs`).
    Chain = 5,
}

impl Kind {
    pub(crate) const fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(Self::Segment),
            2 => Some(Self::Strings),
            3 => Some(Self::Order),
            4 => Some(Self::Messages),
            5 => Some(Self::Chain),
            _ => None,
        }
    }
}

/// Section ids. Fixed forever, a dropped section keeps its slot so old ids never change meaning.
pub mod section {
    pub const COMMITS: u16 = 1;
    pub const FILE_ROWS: u16 = 2;
    pub const PATH_POSTINGS: u16 = 3;
    pub const RENAMES: u16 = 4;
    pub const AUTHOR_POSTINGS: u16 = 5;
    pub const AUTHOR_DAY: u16 = 6;
    /// Path id to life and life to last name, fixed 4 B items (`seg::canon`). Reserved and
    /// empty before format 7, when id 7 was named `MONTH_PATH`: the version bump allowed
    /// reusing it.
    pub const CANON: u16 = 7;
    pub const PATH_AUTHOR: u16 = 8;
    /// Life x local month: touches, line sums, last touch. Reserved and empty before format 7,
    /// when id 9 was named `MONTH_LANGUAGE`.
    pub const PATH_TOUCH: u16 = 9;
    /// Life x identity, the rows of section 8 keyed by life.
    pub const CANON_PATH_AUTHOR: u16 = 10;
    pub const PATHS: u16 = 16;
    pub const IDENTITIES: u16 = 17;
    pub const LABELS: u16 = 18;
    pub const ORDER: u16 = 32;
    pub const RANK: u16 = 33;
    /// Commit records, one per local ordinal.
    pub const ROWS: u16 = 34;
    /// u64 byte offset of each record in the payload of `ROWS`, by local ordinal.
    pub const ROW_INDEX: u16 = 35;
    /// (oid, local ordinal u32) sorted by oid.
    pub const OID_INDEX: u16 = 36;
    /// Message items, zstd blocks, one item per rank (kind 4 only).
    pub const MESSAGES: u16 = 40;
    /// One item per first-parent chain position, newest first (kind 5): rank u32, oid, raw
    /// time i64, offset i32, row count varint, then path id, add and del varints per row.
    pub const CHAIN: u16 = 48;
    /// Chain file meta, written last: order fingerprint u64, commit count u32, length u32.
    pub const CHAIN_META: u16 = 49;
}

/// Cumulative string counts a file was written against, checked on open so ids cannot shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StringBase {
    pub paths: u32,
    pub identities: u32,
    pub labels: u32,
}

/// One block: where its payload is and the crc it must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TocEntry {
    pub section: u16,
    pub items: u32,
    pub offset: u64,
    pub len: u32,
    pub crc: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Footer {
    pub kind: Kind,
    pub toc_offset: u64,
    pub toc_count: u32,
    pub toc_crc: u32,
    pub items: u32,
    pub file_len: u64,
    pub base: StringBase,
}

pub(crate) fn header(kind: Kind) -> [u8; 16] {
    let mut h = [0; 16];
    h[..4].copy_from_slice(&MAGIC);
    h[4..8].copy_from_slice(&VERSION.to_le_bytes());
    h[8..12].copy_from_slice(&(kind as u32).to_le_bytes());
    h
}

pub(crate) fn check_header(bytes: &[u8; 16], kind: Kind, file: &str) -> SegResult<()> {
    if bytes[..4] != MAGIC || bytes[4..8] != VERSION.to_le_bytes() {
        return Err(SegError::unreadable(file, 0, "header magic or version"));
    }
    if bytes[8..12] != (kind as u32).to_le_bytes() || bytes[12..] != [0; 4] {
        return Err(SegError::unreadable(file, 0, "header kind"));
    }
    Ok(())
}

impl Footer {
    pub(crate) fn encode(&self) -> [u8; 64] {
        let mut b = Vec::with_capacity(64);
        b.extend_from_slice(&FOOTER_MAGIC);
        b.extend_from_slice(&VERSION.to_le_bytes());
        b.extend_from_slice(&(self.kind as u32).to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&self.toc_offset.to_le_bytes());
        b.extend_from_slice(&self.toc_count.to_le_bytes());
        b.extend_from_slice(&self.toc_crc.to_le_bytes());
        b.extend_from_slice(&self.items.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&self.file_len.to_le_bytes());
        b.extend_from_slice(&self.base.paths.to_le_bytes());
        b.extend_from_slice(&self.base.identities.to_le_bytes());
        b.extend_from_slice(&self.base.labels.to_le_bytes());
        let crc = crc32fast::hash(&b);
        b.extend_from_slice(&crc.to_le_bytes());
        let mut out = [0; 64];
        out.copy_from_slice(&b);
        out
    }

    /// Parses and checks a footer against the real file length and the expected kind.
    pub(crate) fn decode(b: &[u8; 64], len: u64, kind: Kind, file: &str) -> SegResult<Self> {
        let at = len.saturating_sub(FOOTER_LEN);
        let bad = |what| SegError::unreadable(file, at, what);
        let mut c = Cursor::new(b, file, at);
        let stored = u32::from_le_bytes([b[60], b[61], b[62], b[63]]);
        if crc32fast::hash(&b[..60]) != stored {
            return Err(bad("footer crc"));
        }
        if c.array::<4>()? != FOOTER_MAGIC || c.u32()? != VERSION {
            return Err(bad("footer magic or version"));
        }
        if Kind::from_u32(c.u32()?) != Some(kind) || c.u32()? != 0 {
            return Err(bad("footer kind"));
        }
        let toc_offset = c.u64()?;
        let toc_count = c.u32()?;
        let toc_crc = c.u32()?;
        let items = c.u32()?;
        let zero = c.u32()?;
        let file_len = c.u64()?;
        let base = StringBase {
            paths: c.u32()?,
            identities: c.u32()?,
            labels: c.u32()?,
        };
        if zero != 0 || file_len != len {
            return Err(bad("footer file length"));
        }
        if toc_count as usize > MAX_TOC {
            return Err(bad("toc count over limit"));
        }
        let toc_end = (u64::from(toc_count) * TOC_ENTRY_LEN).checked_add(toc_offset);
        if toc_offset < HEADER_LEN || toc_end != len.checked_sub(FOOTER_LEN) {
            return Err(bad("toc range"));
        }
        Ok(Self {
            kind,
            toc_offset,
            toc_count,
            toc_crc,
            items,
            file_len,
            base,
        })
    }
}

pub(crate) fn encode_toc(entries: &[TocEntry]) -> Vec<u8> {
    let mut b = Vec::with_capacity(entries.len() * TOC_ENTRY_LEN as usize);
    for e in entries {
        b.extend_from_slice(&e.section.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&e.items.to_le_bytes());
        b.extend_from_slice(&e.offset.to_le_bytes());
        b.extend_from_slice(&e.len.to_le_bytes());
        b.extend_from_slice(&e.crc.to_le_bytes());
    }
    b
}

/// Parses a toc whose crc passed, checking every entry against the block area.
pub(crate) fn decode_toc(bytes: &[u8], footer: &Footer, file: &str) -> SegResult<Vec<TocEntry>> {
    if crc32fast::hash(bytes) != footer.toc_crc {
        return Err(SegError::unreadable(file, footer.toc_offset, "toc crc"));
    }
    let mut c = Cursor::new(bytes, file, footer.toc_offset);
    let mut out: Vec<TocEntry> = Vec::with_capacity(footer.toc_count as usize);
    let mut items = 0u64;
    for _ in 0..footer.toc_count {
        let section = c.u16()?;
        let zero = c.u16()?;
        let e = TocEntry {
            section,
            items: c.u32()?,
            offset: c.u64()?,
            len: c.u32()?,
            crc: c.u32()?,
        };
        let end = e.offset.checked_add(u64::from(e.len));
        if zero != 0 || e.len as usize > MAX_BLOCK || e.offset < HEADER_LEN {
            return Err(c.err("toc entry"));
        }
        if end.is_none_or(|end| end > footer.toc_offset) {
            return Err(c.err("toc entry past the block area"));
        }
        if let Some(p) = out.last() {
            let ordered = (p.section, p.offset) < (e.section, e.offset);
            let disjoint = p.section != e.section || p.offset + u64::from(p.len) <= e.offset;
            if !ordered || !disjoint {
                return Err(c.err("toc entries out of order or overlapping"));
            }
        }
        items += u64::from(e.items);
        out.push(e);
    }
    if items != u64::from(footer.items) {
        return Err(SegError::unreadable(
            file,
            footer.toc_offset,
            "toc item total",
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn footer() -> Footer {
        Footer {
            kind: Kind::Segment,
            toc_offset: 40,
            toc_count: 1,
            toc_crc: 7,
            items: 3,
            file_len: 40 + 24 + 64,
            base: StringBase {
                paths: 1,
                identities: 2,
                labels: 3,
            },
        }
    }

    #[test]
    fn footer_round_trips() -> SegResult<()> {
        let f = footer();
        assert_eq!(
            Footer::decode(&f.encode(), f.file_len, Kind::Segment, "t")?,
            f
        );
        Ok(())
    }

    #[test]
    fn every_flipped_footer_byte_is_unreadable() {
        let f = footer();
        let good = f.encode();
        for i in 0..64 {
            let mut b = good;
            b[i] ^= 0x10;
            let r = Footer::decode(&b, f.file_len, Kind::Segment, "t");
            assert!(r.is_err_and(|e| e.is_unreadable()), "byte {i}");
        }
        assert!(Footer::decode(&good, f.file_len + 1, Kind::Segment, "t").is_err());
        assert!(Footer::decode(&good, f.file_len, Kind::Strings, "t").is_err());
    }

    #[test]
    fn hostile_footer_fields_with_valid_crc_are_unreadable() {
        for bad in [
            Footer {
                toc_count: u32::MAX,
                ..footer()
            },
            Footer {
                toc_offset: u64::MAX,
                ..footer()
            },
            Footer {
                toc_offset: 0,
                ..footer()
            },
            Footer {
                file_len: u64::MAX,
                ..footer()
            },
        ] {
            let r = Footer::decode(&bad.encode(), bad.file_len, Kind::Segment, "t");
            assert!(r.is_err_and(|e| e.is_unreadable()), "{bad:?}");
        }
    }

    #[test]
    fn toc_rejects_out_of_range_and_overlap() {
        let mut f = footer();
        let e = TocEntry {
            section: 1,
            items: 3,
            offset: 16,
            len: 24,
            crc: 0,
        };
        let check = |entries: &[TocEntry], f: &mut Footer| {
            let b = encode_toc(entries);
            f.toc_crc = crc32fast::hash(&b);
            f.toc_count = entries.len() as u32;
            f.items = entries.iter().map(|e| e.items).sum();
            decode_toc(&b, f, "t")
        };
        assert!(check(&[e], &mut f).is_ok());
        assert!(check(&[TocEntry { len: 25, ..e }], &mut f).is_err());
        assert!(
            check(
                &[TocEntry {
                    offset: u64::MAX,
                    ..e
                }],
                &mut f
            )
            .is_err()
        );
        assert!(check(&[TocEntry { len: u32::MAX, ..e }], &mut f).is_err());
        let half = TocEntry { len: 12, ..e };
        assert!(check(&[half, TocEntry { offset: 20, ..half }], &mut f).is_err());
        assert!(check(&[half, TocEntry { offset: 28, ..half }], &mut f).is_ok());
    }
}
