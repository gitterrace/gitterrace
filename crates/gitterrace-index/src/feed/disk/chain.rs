//! The disk base's side of the stored chain (`crate::chain`): one step read while writing it,
//! and the whole chain streamed for the languages page.

use crate::feed::disk::DiskSource;
use crate::feed::disk::record::hash32;
use crate::feed::{ChainStep, FileRow, Rank};
use crate::seg::{Oid, SegResult};

impl DiskSource {
    /// One commit as the chain stores it: oid, raw time, offset, first parent, and its file
    /// rows into `files`. One record decode, no oid index lookup.
    pub(crate) fn chain_step(
        &self,
        rank: Rank,
        files: &mut Vec<FileRow>,
    ) -> SegResult<(Oid, i64, i32, Option<Oid>)> {
        let rec = self.decode(rank, Some(files))?;
        Ok((rec.oid, rec.time, rec.offset, rec.parents.first().copied()))
    }

    /// Streams the stored chain newest first, `false` when there is none for this set. A store
    /// error is remembered for `failure` and ends the stream.
    pub(crate) fn stored_chain(&self, each: &mut dyn FnMut(&ChainStep)) -> bool {
        let Some((c, _)) = &self.chain else {
            return false;
        };
        let r = crate::chain::steps(c, self.len, self.counts[0] as usize, self.latest, each);
        self.ok(r);
        true
    }

    /// Streams the stored tip touches, `false` when there are none for this set. A store error
    /// is remembered for `failure` and ends the stream.
    pub(crate) fn stored_tip(&self, each: &mut dyn FnMut(Rank, &[crate::feed::TipTouch])) -> bool {
        let Some(c) = &self.tip else {
            return false;
        };
        let r = crate::tip::read(c, self.len, self.counts[0] as usize, &mut |r, t| {
            each(r, t);
            Ok(())
        });
        self.ok(r);
        true
    }

    pub(crate) const fn stored_has_tip(&self) -> bool {
        self.tip.is_some()
    }

    /// The stored chain's length, checked at open against its steps.
    pub(crate) fn stored_chain_len(&self) -> Option<usize> {
        self.chain.as_ref().map(|(_, n)| *n as usize)
    }

    /// (hash of each path, id), sorted, for `find_path`.
    pub(super) fn path_index(&self) -> SegResult<Vec<(u32, u32)>> {
        let mut v = Vec::with_capacity(self.counts[0].min(1 << 22) as usize);
        for id in 0..self.counts[0] {
            v.push((hash32(&self.string(0, id)?), id));
        }
        v.sort_unstable();
        Ok(v)
    }
}
