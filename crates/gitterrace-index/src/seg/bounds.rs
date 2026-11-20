//! Fixed limits and a checked cursor. The cache may come from someone else's repository, so no
//! field read here sizes a buffer or an offset before it is checked against these limits.

use crate::seg::{SegError, SegResult};

/// Largest block payload.
/// Not configurable, any of these: on-disk format bounds against a corrupt or hostile cache.
/// The writer and every reader must agree on them.
pub(crate) const MAX_BLOCK: usize = 1 << 20;
/// Most table of contents entries in one file, and most files in one manifest.
pub(crate) const MAX_TOC: usize = 4096;
/// Longest string (path, identity, label).
pub(crate) const MAX_STRING: usize = u16::MAX as usize;
/// Longest varint encoding of a u64.
pub(crate) const MAX_VARINT: usize = 10;
pub(crate) const MAX_MANIFEST: u64 = 1 << 20;

/// Reads little-endian fields from a slice whose crc already passed. Every read is checked.
pub(crate) struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
    file: &'a str,
    base: u64,
}

impl<'a> Cursor<'a> {
    /// `base` is the slice's offset in `file`, for the error message only.
    pub(crate) const fn new(buf: &'a [u8], file: &'a str, base: u64) -> Self {
        Self { buf, pos: 0, file, base }
    }

    pub(crate) fn err(&self, what: &'static str) -> SegError {
        SegError::unreadable(self.file, self.base.saturating_add(self.pos as u64), what)
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.pos >= self.buf.len()
    }

    pub(crate) fn bytes(&mut self, n: usize) -> SegResult<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len());
        let end = end.ok_or_else(|| self.err("field past the end"))?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub(crate) fn array<const N: usize>(&mut self) -> SegResult<[u8; N]> {
        let mut out = [0; N];
        out.copy_from_slice(self.bytes(N)?);
        Ok(out)
    }

    pub(crate) fn u8(&mut self) -> SegResult<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub(crate) fn u16(&mut self) -> SegResult<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(crate) fn u32(&mut self) -> SegResult<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(crate) fn u64(&mut self) -> SegResult<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    /// LEB128, at most [`MAX_VARINT`] bytes, no bits past 64.
    pub(crate) fn varint(&mut self) -> SegResult<u64> {
        let mut out = 0u64;
        for i in 0..MAX_VARINT {
            let b = self.u8()?;
            let low = u64::from(b & 0x7f);
            if i == MAX_VARINT - 1 && low > 1 {
                return Err(self.err("varint overflows u64"));
            }
            out |= low << (7 * i);
            if b & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(self.err("varint longer than 10 bytes"))
    }

    /// A count that must fit `max`, for sizing a buffer.
    pub(crate) fn count(&mut self, max: usize) -> SegResult<usize> {
        let n = self.varint()?;
        usize::try_from(n).ok().filter(|&n| n <= max).ok_or_else(|| self.err("count over limit"))
    }
}

/// Appends a LEB128 varint.
pub(crate) fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

pub(crate) const fn zigzag(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

pub(crate) const fn unzigzag(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(bytes: &[u8]) -> SegResult<u64> {
        Cursor::new(bytes, "t", 0).varint()
    }

    #[test]
    fn varint_round_trips() {
        for v in [0, 1, 127, 128, 300, u64::from(u32::MAX), u64::MAX] {
            let mut b = Vec::new();
            put_varint(&mut b, v);
            assert!(b.len() <= MAX_VARINT);
            assert_eq!(read(&b).ok(), Some(v));
        }
        for v in [0, -1, 1, i64::MIN, i64::MAX] {
            assert_eq!(unzigzag(zigzag(v)), v);
        }
    }

    #[test]
    fn hostile_varints_are_unreadable() {
        assert!(read(&[0x80; 11]).is_err_and(|e| e.is_unreadable()));
        assert!(read(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02]).is_err());
        assert!(read(&[0x80]).is_err());
        assert!(read(&[]).is_err());
    }

    #[test]
    fn reads_past_end_and_huge_counts_fail() {
        let mut c = Cursor::new(&[1, 2, 3], "t", 0);
        assert!(c.u32().is_err());
        let mut b = Vec::new();
        put_varint(&mut b, u64::MAX);
        assert!(Cursor::new(&b, "t", 0).count(MAX_TOC).is_err());
        let mut c = Cursor::new(&[0; 4], "t", 0);
        assert!(c.bytes(usize::MAX).is_err());
    }
}
