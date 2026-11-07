//! The segment store: immutable container files named by one manifest.
//!
//! Shared ids live here so the feed trait and the aggregates agree on them.
//! The format, memory model and build path: `store.md` beside this file.

pub(crate) mod agg;
pub(crate) mod bounds;
pub mod canon;
pub mod format;
pub(crate) mod io;
pub mod manifest;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "every update writes one fresh segment, so nothing merges yet (step 2)"
    )
)]
pub(crate) mod merge;
pub(crate) mod postings;
pub mod read;
pub(crate) mod spill;
pub mod strings;
pub mod write;

use std::fmt;

/// A commit id, raw bytes.
pub type Oid = [u8; 20];

/// A commit's position inside one segment. Never compared across segments.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Ordinal(pub u32);

/// A commit's position in the order file: the canonical order of the whole reachable set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Rank(pub u32);

/// A path, cumulative over the string deltas in manifest order, so stable once written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PathId(pub u32);

/// A raw identity (author and email as written, before mailmap). Cumulative like [`PathId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct IdentId(pub u32);

/// A conventional-commit kind or scope name. Cumulative like [`PathId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct LabelId(pub u32);

/// Every failure of the store. `Unreadable` is mapped to a rebuild by the caller, never to an
/// HTTP error.
#[derive(Debug, thiserror::Error)]
pub enum SegError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("unreadable cache file {file} at offset {offset}: {what}")]
    Unreadable {
        file: String,
        offset: u64,
        what: &'static str,
    },
    #[error("{kind} of {len} bytes exceeds the {max} byte limit")]
    TooLong {
        kind: &'static str,
        len: usize,
        max: usize,
    },
    #[error("invalid cache file name {0:?}")]
    BadName(String),
}

impl SegError {
    pub(crate) fn unreadable(file: impl fmt::Display, offset: u64, what: &'static str) -> Self {
        Self::Unreadable {
            file: file.to_string(),
            offset,
            what,
        }
    }

    /// True when the cache must be deleted and rebuilt.
    pub const fn is_unreadable(&self) -> bool {
        matches!(self, Self::Unreadable { .. })
    }
}

pub type SegResult<T> = Result<T, SegError>;

/// Receives one block: section id, item count, payload.
pub(crate) type Sink<'s> = dyn FnMut(u16, u32, &[u8]) -> SegResult<()> + 's;

/// FNV-1a 64 over the ordered oid bytes: the manifest's order fingerprint.
pub fn fingerprint<'a>(oids: impl IntoIterator<Item = &'a Oid>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for oid in oids {
        for &b in oid {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_depends_on_order() {
        let (x, y) = ([1; 20], [2; 20]);
        assert_ne!(fingerprint([&x, &y]), fingerprint([&y, &x]));
        assert_eq!(fingerprint([]), 0xcbf2_9ce4_8422_2325);
    }
}
