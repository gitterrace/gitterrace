//! External sort for path postings of a big build (linux): entries collect in memory and are
//! cut into sorted runs in a temp file once over a fixed entry count, then merged k-way at the
//! end. Runs are cut by count, never by time or thread, so the output is a pure function of
//! the input.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};

use crate::seg::io::read_exact_at;

/// 128 MB of 12-byte entries.
pub(crate) const RUN_ENTRIES: usize = 128 * 1024 * 1024 / ENTRY;
const ENTRY: usize = 12;
/// Entries read per run at a time (64 KiB buffers).
const READ_ENTRIES: u64 = 64 * 1024 / ENTRY as u64;

/// (path id, ordinal, row): sorted by all three, so per path the hits come out ascending.
pub(crate) type Entry = (u32, u32, u32);

pub(crate) struct Spill {
    buf: Vec<Entry>,
    limit: usize,
    /// Opens the run file on the first cut (the caller owns where it lives and its removal).
    open: Box<dyn FnMut() -> io::Result<File>>,
    file: Option<File>,
    /// (start, entries) per run.
    runs: Vec<(u64, u64)>,
    end: u64,
}

impl Spill {
    /// The build's spill: runs of [`RUN_ENTRIES`] in a file `open` creates.
    pub(crate) fn for_build(open: impl FnMut() -> io::Result<File> + 'static) -> Self {
        Self::new(RUN_ENTRIES, open)
    }

    pub(crate) fn new(limit: usize, open: impl FnMut() -> io::Result<File> + 'static) -> Self {
        Self {
            buf: Vec::new(),
            limit: limit.max(1),
            open: Box::new(open),
            file: None,
            runs: Vec::new(),
            end: 0,
        }
    }

    pub(crate) fn push(&mut self, e: Entry) -> io::Result<()> {
        self.buf.push(e);
        if self.buf.len() >= self.limit {
            self.cut()?;
        }
        Ok(())
    }

    /// Runs written so far (a test hook).
    #[cfg_attr(not(test), expect(dead_code, reason = "a test hook"))]
    pub(crate) const fn runs(&self) -> usize {
        self.runs.len()
    }

    fn cut(&mut self) -> io::Result<()> {
        self.buf.sort_unstable();
        let file = match &mut self.file {
            Some(f) => f,
            None => self.file.insert((self.open)()?),
        };
        file.seek(SeekFrom::Start(self.end))?;
        let mut w = BufWriter::new(&mut *file);
        for &(a, b, c) in &self.buf {
            w.write_all(&a.to_le_bytes())?;
            w.write_all(&b.to_le_bytes())?;
            w.write_all(&c.to_le_bytes())?;
        }
        w.flush()?;
        let n = self.buf.len() as u64;
        self.runs.push((self.end, n));
        self.end += n * ENTRY as u64;
        self.buf.clear();
        Ok(())
    }

    /// Every entry ascending, through `f`. Memory is one buffered reader per run.
    pub(crate) fn finish(mut self, mut f: impl FnMut(Entry) -> io::Result<()>) -> io::Result<()> {
        if self.runs.is_empty() {
            self.buf.sort_unstable();
            for e in self.buf {
                f(e)?;
            }
            return Ok(());
        }
        if !self.buf.is_empty() {
            self.cut()?;
        }
        let Some(file) = &self.file else {
            return Ok(());
        };
        // One file handle, one buffer per run, positioned reads: fan-in costs no descriptors.
        let mut runs: Vec<Run> = self
            .runs
            .iter()
            .map(|&(start, n)| Run { pos: start, left: n, buf: Vec::new(), at: 0 })
            .collect();
        let mut heap = BinaryHeap::new();
        for (i, r) in runs.iter_mut().enumerate() {
            if let Some(e) = r.next(file)? {
                heap.push(Reverse((e, i)));
            }
        }
        while let Some(Reverse((e, i))) = heap.pop() {
            f(e)?;
            if let Some(n) = runs[i].next(file)? {
                heap.push(Reverse((n, i)));
            }
        }
        Ok(())
    }
}

struct Run {
    pos: u64,
    left: u64,
    buf: Vec<u8>,
    at: usize,
}

impl Run {
    fn next(&mut self, file: &File) -> io::Result<Option<Entry>> {
        if self.at == self.buf.len() {
            if self.left == 0 {
                return Ok(None);
            }
            let n = self.left.min(READ_ENTRIES);
            self.buf.resize(usize::try_from(n).unwrap_or(0) * ENTRY, 0);
            read_exact_at(file, &mut self.buf, self.pos)?;
            self.pos += n * ENTRY as u64;
            self.left -= n;
            self.at = 0;
        }
        let b = &self.buf[self.at..self.at + ENTRY];
        self.at += ENTRY;
        let w = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        Ok(Some((w(0), w(4), w(8))))
    }
}

#[cfg(test)]
mod tests {
    use super::{Entry, Spill};

    fn input() -> Vec<Entry> {
        let mut x: u32 = 0x9e37_79b9;
        (0..10_000)
            .map(|i| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x % 97, i, x % 7)
            })
            .collect()
    }

    fn run(limit: usize) -> (Vec<Entry>, usize) {
        let mut s = Spill::new(limit, tempfile::tempfile);
        for e in input() {
            assert!(s.push(e).is_ok());
        }
        let runs = s.runs();
        let mut out = Vec::new();
        let r = s.finish(|e| {
            out.push(e);
            Ok(())
        });
        assert!(r.is_ok(), "limit {limit}: {r:?}");
        (out, runs)
    }

    #[test]
    fn spilled_output_equals_in_memory_sort() {
        let mut want = input();
        want.sort_unstable();
        let (mem, r0) = run(usize::MAX);
        assert_eq!(r0, 0);
        assert_eq!(mem, want);
        for limit in [1, 7, 1000, 9999] {
            let (out, runs) = run(limit);
            assert!(runs > 0, "limit {limit} never spilled");
            assert_eq!(out, want, "limit {limit}");
        }
    }
}
