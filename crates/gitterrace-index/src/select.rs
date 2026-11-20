//! Which source a reader of a rev gets: the set when it holds the rev exactly, else the reach
//! view over it. Web pages and the API both pick through [`Index::source_for`].

use std::collections::HashSet;
use std::io;

use crate::{CommitSource, DiskSet, Index, TIP_FILE, cache, feed, seg, store};

/// What [`Index::source_for`] picked for a rev.
pub enum Selected {
    /// The set, or the reach view over it, behind one checked handle.
    Ready(feed::Checked<dyn feed::InsightsSource>),
    /// The rev lacks more than [`Index::fresh_cap`] commits the set holds: reindex for it.
    NeedsReindex { read: usize },
}

impl Index {
    /// What pages read for `rev`: the segment files alone (aggregates, postings, edges from
    /// [`feed::SegmentSource`], commits, rows and strings from [`feed::DiskSource`]), so no
    /// history is held. The set must be exactly `rev`'s canonical order (the manifest holds
    /// one order, the last updated rev). `None` when it is not, or when there is no set yet:
    /// the caller then tries [`Index::rev_source`].
    pub fn disk_source(
        &self,
        source: &dyn CommitSource,
        rev: &str,
    ) -> io::Result<Option<feed::Checked<DiskSet>>> {
        // A store error opening the set is an unreadable cache, which callers rebuild.
        let Some(store) = seg::read::Store::open(&self.dir).map_err(feed::SourceError::io)? else {
            return Ok(None);
        };
        // The set was built for this exact commit: no history walk needed to prove it holds it.
        let built = cache::read_stamp(&self.dir, TIP_FILE)
            .ok()
            .flatten()
            .is_some_and(|t| t.trim_ascii() == rev.as_bytes() && store::oid_of(rev).is_some());
        let order = if built { Vec::new() } else { source.rev_list(rev)? };
        let m = &store.manifest;
        // An unmerged set's order file is old order then new commits, not canonical, so the
        // fingerprint only matches after a merge: compare the sets.
        let same = built
            || m.commits as usize == order.len()
                && store::load_order(&store).is_ok_and(|held| {
                    let held: HashSet<seg::Oid> = held.into_iter().collect();
                    order.iter().all(|id| store::oid_of(id).is_some_and(|o| held.contains(&o)))
                });
        if !same {
            return Ok(None);
        }
        Ok(self
            .set_over(store, source, rev)?
            .map(|set| feed::Checked::new(std::sync::Arc::new(set))))
    }

    /// The segment set over the rows of `rows`, identities from `rev`'s `.mailmap`, with a
    /// second handle on the store for the tables. `None` when the set went away in between.
    fn set_over(
        &self,
        rows: seg::read::Store,
        source: &dyn CommitSource,
        rev: &str,
    ) -> io::Result<Option<DiskSet>> {
        // A store error opening the set is an unreadable cache, which callers rebuild.
        let seg = feed::SourceError::io;
        let disk = feed::DiskSource::open(rows, &mut |pairs| source.map_identities(rev, pairs))
            .map_err(seg)?
            .with_cache_bytes(self.block_cache.load(std::sync::atomic::Ordering::Relaxed));
        let Some(tables) = seg::read::Store::open(&self.dir).map_err(seg)? else {
            return Ok(None);
        };
        Ok(Some(feed::SegmentSource::new(disk, tables.segments)))
    }

    /// The one selection every reader of `rev` (resolved to `tip`) makes: the set when it holds
    /// `rev` exactly ([`Index::disk_source`]), else the reach view over it
    /// ([`Index::rev_source`]). `None` without a set: what to show instead (build first, or the
    /// indexing page) is the caller's, and so is what a [`Selected::NeedsReindex`] means.
    pub fn source_for(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        tip: &str,
    ) -> io::Result<Option<Selected>> {
        if let Some(set) = self.disk_source(source, rev)? {
            return Ok(Some(Selected::Ready(set.erased())));
        }
        Ok(self.rev_source(source, rev, tip)?.map(|view| match view {
            feed::RevView::Ready(reach) => Selected::Ready(reach.erased()),
            feed::RevView::NeedsReindex { read } => Selected::NeedsReindex { read },
        }))
    }

    /// The view of a commit the set does not hold exactly: the set's commits it reaches plus
    /// up to [`Index::fresh_cap`] it lacks, read from git. `tip` is `rev` resolved to a commit
    /// id. Past the cap the answer is [`feed::RevView::NeedsReindex`]: building the set for
    /// this commit would move it off HEAD, and holding its records would break the memory
    /// bound. `None` when there is no set.
    pub fn rev_source(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        tip: &str,
    ) -> io::Result<Option<feed::RevView<DiskSet>>> {
        // A store error opening the set is an unreadable cache, which callers rebuild.
        let Some(rows) = seg::read::Store::open(&self.dir).map_err(feed::SourceError::io)? else {
            return Ok(None);
        };
        let Some(set) = self.set_over(rows, source, rev)? else {
            return Ok(None);
        };
        feed::rev_view(set, source, rev, tip, self.fresh_cap).map(Some)
    }
}
