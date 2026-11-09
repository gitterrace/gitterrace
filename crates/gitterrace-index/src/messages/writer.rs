//! The message column writer: blocks of items, zstd per block, rolling to the next file
//! when a container's toc is full.

use std::path::Path;

use crate::messages::{BLOCK_RAW, Item, codec};
use crate::seg::SegResult;
use crate::seg::bounds::MAX_TOC;
use crate::seg::format::{Kind, StringBase, section};
use crate::seg::manifest::FileEntry;
use crate::seg::write::ContainerWriter;

/// Writes the message files of one step.
///
/// The first file is `msg-<gen>-<n>.gts`, then `-<n+1>` and on once a file holds
/// [`MAX_TOC`] blocks, the most a container's toc takes. Readers take the files in manifest
/// order, so a column of any length is several files. Drop without `finish` leaves only
/// `.tmp` files, which the sweep removes.
pub struct Writer {
    dir: std::path::PathBuf,
    generation: u64,
    next: usize,
    w: ContainerWriter,
    /// Blocks in `w`, and the most one file takes.
    blocks: usize,
    limit: usize,
    /// Blocks written over all files.
    written: usize,
    done: Vec<FileEntry>,
    raw: Vec<u8>,
    items: u32,
}

impl Writer {
    /// Starts with file `msg-<generation>-<first>.gts`.
    pub fn new(dir: &Path, generation: u64, first: usize) -> SegResult<Self> {
        let limit = toc_limit();
        Ok(Self {
            dir: dir.to_path_buf(),
            generation,
            next: first + 1,
            w: open_file(dir, generation, first)?,
            blocks: 0,
            limit: limit.max(1),
            written: 0,
            done: Vec::new(),
            raw: Vec::with_capacity(BLOCK_RAW * 2),
            items: 0,
        })
    }

    pub fn push(&mut self, item: &Item) -> SegResult<()> {
        item.encode(&mut self.raw);
        self.items += 1;
        if self.raw.len() >= BLOCK_RAW {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> SegResult<()> {
        if self.items == 0 {
            return Ok(());
        }
        if self.blocks == self.limit {
            let full = std::mem::replace(
                &mut self.w,
                open_file(&self.dir, self.generation, self.next)?,
            );
            self.next += 1;
            self.blocks = 0;
            self.done.push(full.finish()?);
        }
        #[cfg(test)]
        if TEST_FAIL_AT.with(|f| f.get() == Some(self.written)) {
            return Err(crate::seg::SegError::BadName("test: block refused".into()));
        }
        self.written += 1;
        // Cleared before the write: a failed block never makes the next one bigger.
        let z = codec::pack(&self.raw);
        self.raw.clear();
        let items = std::mem::take(&mut self.items);
        self.w.block(section::MESSAGES, items, &z?)?;
        self.blocks += 1;
        Ok(())
    }

    /// The files written, in rank order, to name in the manifest in this order.
    pub fn finish(mut self) -> SegResult<Vec<FileEntry>> {
        self.flush()?;
        self.done.push(self.w.finish()?);
        Ok(self.done)
    }
}

/// Blocks per file: the container's toc bound, smaller in a test so a few commits roll.
#[cfg(not(test))]
const fn toc_limit() -> usize {
    MAX_TOC
}

#[cfg(test)]
fn toc_limit() -> usize {
    TEST_TOC.with(std::cell::Cell::get).unwrap_or(MAX_TOC)
}

#[cfg(test)]
thread_local! {
    /// Set by a test on its own thread, see [`toc_limit`].
    pub(crate) static TEST_TOC: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    /// The block (counted over all files) a test makes fail.
    pub(crate) static TEST_FAIL_AT: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
}

fn open_file(dir: &Path, generation: u64, n: usize) -> SegResult<ContainerWriter> {
    let name = format!("msg-{generation}-{n}.gts");
    ContainerWriter::new(dir, &name, Kind::Messages, StringBase::default())
}
