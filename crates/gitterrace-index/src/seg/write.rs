//! Writes one immutable container.
//!
//! Blocks streamed to `<name>.tmp`, then toc and footer, fsync, rename, directory sync. Before the
//! rename the file does not exist for readers. The canonical order itself is
//! `gitterrace_git::canonical_order`, the one definition: writers take the ordered oids as input.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::seg::bounds::{MAX_BLOCK, MAX_TOC};
use crate::seg::format::{
    FOOTER_LEN, Footer, HEADER_LEN, Kind, StringBase, TocEntry, encode_toc, header,
};
use crate::seg::io::{create_no_follow, sync_dir};
use crate::seg::manifest::{FileEntry, Step, check_name};
use crate::seg::{Oid, SegError, SegResult};

/// Streams blocks into a new container. Drop without `finish` leaves only a `.tmp`, which the
/// sweep removes.
pub struct ContainerWriter {
    dir: PathBuf,
    name: String,
    kind: Kind,
    base: StringBase,
    out: BufWriter<File>,
    offset: u64,
    toc: Vec<TocEntry>,
    items: u32,
}

impl ContainerWriter {
    /// `base` is the cumulative string count this file is written against (zero for order).
    pub fn new(dir: &Path, name: &str, kind: Kind, base: StringBase) -> SegResult<Self> {
        check_name(name)?;
        let mut out = BufWriter::new(create_no_follow(dir, &format!("{name}.tmp"))?);
        out.write_all(&header(kind))?;
        Ok(Self {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            kind,
            base,
            out,
            offset: HEADER_LEN,
            toc: Vec::new(),
            items: 0,
        })
    }

    /// Appends one block. Blocks of a section must be written in order and sections in id
    /// order, which the reader checks.
    pub fn block(&mut self, section: u16, items: u32, payload: &[u8]) -> SegResult<()> {
        if payload.len() > MAX_BLOCK {
            return Err(SegError::TooLong {
                kind: "block",
                len: payload.len(),
                max: MAX_BLOCK,
            });
        }
        if self.toc.len() >= MAX_TOC {
            return Err(SegError::TooLong {
                kind: "toc",
                len: self.toc.len() + 1,
                max: MAX_TOC,
            });
        }
        if self.toc.last().is_some_and(|l| l.section > section) {
            return Err(SegError::BadName(format!(
                "section {section} after a higher one"
            )));
        }
        let items_total = self.items.checked_add(items);
        let items_total = items_total.ok_or(SegError::TooLong {
            kind: "item total",
            len: usize::MAX,
            max: u32::MAX as usize,
        })?;
        let len = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        self.out.write_all(payload)?;
        let crc = crc32fast::hash(payload);
        self.toc.push(TocEntry {
            section,
            items,
            offset: self.offset,
            len,
            crc,
        });
        self.offset += u64::from(len);
        self.items = items_total;
        Ok(())
    }

    /// Writes toc and footer, fsyncs and renames into place.
    pub fn finish(self) -> SegResult<FileEntry> {
        self.finish_with(&mut |_| true)
    }

    /// As `finish`, but `hook` sees each step after it happened and stops the sequence by
    /// returning false, which is how tests simulate a crash.
    pub fn finish_with(mut self, hook: &mut dyn FnMut(Step) -> bool) -> SegResult<FileEntry> {
        let toc = encode_toc(&self.toc);
        let toc_count = u32::try_from(self.toc.len()).unwrap_or(u32::MAX);
        let file_len = self.offset + toc.len() as u64 + FOOTER_LEN;
        let footer = Footer {
            kind: self.kind,
            toc_offset: self.offset,
            toc_count,
            toc_crc: crc32fast::hash(&toc),
            items: self.items,
            file_len,
            base: self.base,
        };
        let footer = footer.encode();
        self.out.write_all(&toc)?;
        self.out.write_all(&footer)?;
        let file = self
            .out
            .into_inner()
            .map_err(|e| SegError::Io(e.into_error()))?;
        file.sync_all()?;
        drop(file);
        if !hook(Step::FileWritten(self.name.clone())) {
            return Err(crashed());
        }
        let tmp = self.dir.join(format!("{}.tmp", self.name));
        fs::rename(tmp, self.dir.join(&self.name))?;
        sync_dir(&self.dir)?;
        if !hook(Step::FileRenamed(self.name.clone())) {
            return Err(crashed());
        }
        Ok(FileEntry {
            kind: self.kind as u8,
            name: self.name,
            len: file_len,
            footer_crc: crc32fast::hash(&footer),
        })
    }
}

pub(crate) fn crashed() -> SegError {
    SegError::Io(std::io::Error::other("simulated crash"))
}

/// Writes the order file: every reachable oid in the order given, which is the canonical order
/// from `gitterrace_git::canonical_order`, 20 B each.
pub fn write_order(dir: &Path, name: &str, oids: &[Oid]) -> SegResult<FileEntry> {
    let mut w = ContainerWriter::new(dir, name, Kind::Order, StringBase::default())?;
    for chunk in oids.chunks(MAX_BLOCK / 20) {
        let bytes: Vec<u8> = chunk.iter().flatten().copied().collect();
        let n = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
        w.block(crate::seg::format::section::ORDER, n, &bytes)?;
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_writer_inputs_error_not_panic() -> SegResult<()> {
        let dir = tempfile::tempdir()?;
        let mut w = ContainerWriter::new(
            dir.path(),
            "seg-1-0.gts",
            Kind::Segment,
            StringBase::default(),
        )?;
        assert!(w.block(1, 1, &vec![0; MAX_BLOCK + 1]).is_err());
        w.block(2, 1, b"x")?;
        assert!(w.block(1, 1, b"x").is_err());
        assert!(
            ContainerWriter::new(dir.path(), "../x", Kind::Segment, StringBase::default()).is_err()
        );
        Ok(())
    }
}
