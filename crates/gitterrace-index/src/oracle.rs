//! The equivalence oracle's helpers: records read back whole, as pages would see them.

use std::io;

use crate::{CommitRecord, CommitSource, feed};

/// "Now" for period windows over records, the oracle's twin of [`feed::reference_time`]: the
/// newest author time not after this machine's clock.
///
/// So an old repository still shows its last active stretch and a commit dated in the future
/// (one [`crate::plausible_time`] pinned to the edge, or a skewed clock) never empties every
/// window. Every surface that cuts a period uses this one value.
pub fn reference_time(records: &[CommitRecord]) -> i64 {
    reference_time_at(records, &feed::SystemClock)
}

/// [`reference_time`] with the clock given.
pub fn reference_time_at(records: &[CommitRecord], clock: &dyn feed::Clock) -> i64 {
    let wall = clock.now();
    records.iter().map(|r| r.time).filter(|t| *t <= wall).max().unwrap_or(0)
}

/// Rewrites every record's author through the source's identity map, asking once per
/// distinct pair.
pub(crate) fn map_identities(
    source: &dyn CommitSource,
    rev: &str,
    records: &mut [CommitRecord],
) -> io::Result<()> {
    let (mapped, slot) = crate::source::mapped_pairs(source, rev, records)?;
    for r in records.iter_mut() {
        let key = (std::mem::take(&mut r.author), std::mem::take(&mut r.email));
        let (name, email) = &mapped[slot[&key]];
        r.author.clone_from(name);
        r.email.clone_from(email);
    }
    Ok(())
}
