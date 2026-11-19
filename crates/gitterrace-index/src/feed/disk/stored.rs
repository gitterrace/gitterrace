//! The disk base's side of the stored tip touches (`crate::tip`) and the path index.

use crate::feed::Rank;
use crate::feed::disk::DiskSource;
use crate::feed::disk::record::hash32;
use crate::seg::SegResult;

impl DiskSource {
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
