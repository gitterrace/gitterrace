//! The row writer: spools commit records and emits the row sections of one segment.

use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use crate::CommitRecord;
use crate::feed::disk::record::{oid, oid_of};
use crate::feed::disk::{MAX_RECORD, OID_ENTRY, Sink, columns};
use crate::seg::bounds::MAX_BLOCK;
use crate::seg::format::section;
use crate::seg::{Oid, SegError, SegResult};

fn encode(out: &mut Vec<u8>, r: &CommitRecord, w: &Ids<'_>) -> SegResult<()> {
    let oid = oid(&r.id)?;
    out.extend_from_slice(&oid);
    out.extend_from_slice(&w.who.to_le_bytes());
    out.extend_from_slice(&r.time.to_le_bytes());
    out.extend_from_slice(&r.offset.to_le_bytes());
    out.push(u8::from(r.merge));
    let opt = |v: Option<u32>| v.map_or(0, |v| v + 1);
    out.extend_from_slice(&opt(w.kind).to_le_bytes());
    out.extend_from_slice(&opt(w.scope).to_le_bytes());
    let np = u16::try_from(r.parents.len()).map_err(|_| SegError::TooLong {
        kind: "parent list",
        len: r.parents.len(),
        max: u16::MAX as usize,
    })?;
    out.extend_from_slice(&np.to_le_bytes());
    for p in &r.parents {
        out.extend_from_slice(&oid_of(p)?);
    }
    if w.paths.len() != r.files.len() || w.olds.len() != r.files.len() {
        return Err(SegError::BadName("row ids do not match the files".into()));
    }
    let nf = u32::try_from(r.files.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&nf.to_le_bytes());
    for ((f, &p), &o) in r.files.iter().zip(w.paths).zip(w.olds) {
        for v in [p, opt(o), f.additions, f.deletions, f.lines, f.rewritten] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.push(u8::from(f.binary) | (u8::from(f.code.is_some()) << 1));
        if let Some(c) = &f.code {
            for v in [c.additions, c.deletions, c.rewritten] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(())
}

struct Ids<'a> {
    who: u32,
    kind: Option<u32>,
    scope: Option<u32>,
    paths: &'a [u32],
    olds: &'a [Option<u32>],
}

/// Writes the row sections of one segment.
///
/// Records are spooled to `rows-<gen>.gts.tmp` in
/// the cache directory while the other sections are built (sections ascend in a container),
/// then copied into blocks by [`RowWriter::finish`]. A crash leaves only the `.tmp`, which
/// the open sweep removes, and the spool is never named in the manifest.
pub struct RowWriter {
    path: PathBuf,
    spool: BufWriter<File>,
    /// Section 1, one fixed item per commit, spooled until [`RowWriter::commits`].
    cols: columns::Spool,
    offsets: Vec<u64>,
    oids: Vec<(Oid, u32)>,
    len: u64,
    buf: Vec<u8>,
}

impl RowWriter {
    pub fn new(dir: &Path, generation: u64) -> SegResult<Self> {
        let path = dir.join(format!("rows-{generation}.gts.tmp"));
        let spool = BufWriter::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?,
        );
        let cols = columns::Spool::new(&dir.join(format!("cols-{generation}.gts.tmp")))?;
        Ok(Self {
            path,
            spool,
            cols,
            offsets: Vec::new(),
            oids: Vec::new(),
            len: 0,
            buf: Vec::new(),
        })
    }

    /// Appends the next record (the next local ordinal). Ids are the cache's global ids, one
    /// path and old path per file row. A NUL in the author or email is refused.
    pub fn push(
        &mut self,
        r: &CommitRecord,
        who: u32,
        kind: Option<u32>,
        scope: Option<u32>,
        paths: &[u32],
        olds: &[Option<u32>],
    ) -> SegResult<()> {
        if r.author.contains('\0') || r.email.contains('\0') {
            return Err(SegError::BadName("NUL in an author name or email".into()));
        }
        self.buf.clear();
        let ids = Ids {
            who,
            kind,
            scope,
            paths,
            olds,
        };
        encode(&mut self.buf, r, &ids)?;
        if self.buf.len() > MAX_RECORD {
            return Err(SegError::TooLong {
                kind: "commit record",
                len: self.buf.len(),
                max: MAX_RECORD,
            });
        }
        self.cols.push(&columns::item(r, who, kind, scope))?;
        let ord = u32::try_from(self.offsets.len()).unwrap_or(u32::MAX);
        self.oids.push((oid(&r.id)?, ord));
        self.offsets.push(self.len);
        self.spool.write_all(&self.buf)?;
        self.len += self.buf.len() as u64;
        Ok(())
    }

    /// Writes section 1 (the commit columns) through `sink`. Call first, before any other
    /// section of the segment: sections go in id order.
    pub fn commits(&mut self, sink: &mut Sink<'_>) -> SegResult<()> {
        self.cols.finish(sink)
    }

    /// Writes sections 34, 35 and 36 through `sink`, after section 33. Removes the spool.
    pub fn finish(self, sink: &mut Sink<'_>) -> SegResult<()> {
        let Self {
            path,
            spool,
            offsets,
            mut oids,
            len,
            ..
        } = self;
        let mut file = spool
            .into_inner()
            .map_err(|e| SegError::Io(e.into_error()))?;
        std::io::Seek::rewind(&mut file)?;
        let mut block = vec![0u8; MAX_BLOCK];
        let (mut pos, mut next) = (0u64, 0usize);
        while pos < len {
            let n = usize::try_from((len - pos).min(MAX_BLOCK as u64)).unwrap_or(MAX_BLOCK);
            file.read_exact(&mut block[..n])?;
            let end = pos + n as u64;
            let mut items = 0u32;
            while next < offsets.len() && offsets[next] < end {
                items += 1;
                next += 1;
            }
            sink(section::ROWS, items, &block[..n])?;
            pos = end;
        }
        drop(file);
        drop(std::fs::remove_file(&path));
        for chunk in offsets.chunks(MAX_BLOCK / 8) {
            let bytes: Vec<u8> = chunk.iter().flat_map(|o| o.to_le_bytes()).collect();
            sink(
                section::ROW_INDEX,
                u32::try_from(chunk.len()).unwrap_or(0),
                &bytes,
            )?;
        }
        oids.sort_unstable();
        for chunk in oids.chunks(MAX_BLOCK / OID_ENTRY) {
            let mut bytes = Vec::with_capacity(chunk.len() * OID_ENTRY);
            for (o, ord) in chunk {
                bytes.extend_from_slice(o);
                bytes.extend_from_slice(&ord.to_le_bytes());
            }
            sink(
                section::OID_INDEX,
                u32::try_from(chunk.len()).unwrap_or(0),
                &bytes,
            )?;
        }
        Ok(())
    }
}
