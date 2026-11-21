//! The write path: an index update under the writer lock, and the segment merge after it.

use std::collections::HashSet;
use std::io;

use crate::{
    BATCH, CommitSource, Index, Progress, TIP_FILE, cache, seg, seg_io, skipped, store, tip, todo,
};

impl Index {
    /// Indexes commits reachable from `rev` that the segment set lacks: one more segment when
    /// every commit the set holds is still reachable, else a fresh build streamed from the
    /// source. A deepened or grafted clone wipes the set first (see `check_shallow`), so a
    /// boundary commit is diffed again with its parents. Waits for any other process doing the
    /// same; a commit the source cannot read is skipped and stays pending.
    pub fn update(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        progress: &dyn Fn(Progress),
    ) -> io::Result<()> {
        if let Some((order, shallow)) = self.update_held(source, rev, progress)?
            && !self.halted()
        {
            // After the writer lock is gone: the merge takes it too, and an inline merge under
            // the update's own lock would wait on itself forever.
            self.merge_in_background(order, shallow);
        }
        Ok(())
    }

    /// [`Index::update`] under the writer lock. Returns the order and shallow ids for a
    /// segment merge when the set grew past [`store::MAX_SEGMENTS`].
    fn update_held(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        progress: &dyn Fn(Progress),
    ) -> io::Result<Option<(Vec<String>, Vec<String>)>> {
        self.writable()?;
        let ph = &self.phases;
        ph.reset();
        ph.start("rev_list");
        let tip = source.resolve(rev);
        let order = source.rev_list(rev)?;
        let shallow = source.shallow_ids();
        ph.start("held");
        // A deepened or grafted clone changes topo ranks: same rule as an unreadable cache.
        store::check_shallow(&self.dir, &shallow)?;
        // Everything below reads the set another writer may be extending: lock first.
        let _lock = store::WriteLock::take(&self.dir)?;
        // Gone before the set can change, written back once it holds exactly `tip`: a crash in
        // between leaves no marker, so the next open checks the set the slow way.
        cache::remove(&self.dir, TIP_FILE)?;
        let seg = seg_io;
        let previous = seg::manifest::Manifest::load(&self.dir).map_err(seg)?;
        let held = self.held()?;
        let todo = todo(&order, &held);
        if todo.is_empty() {
            self.sync_side(source)?;
            if held.len() == order.len()
                && let Some(t) = tip
            {
                cache::write_atomic(&self.dir, TIP_FILE, t.as_bytes())?;
            }
            return Ok(None);
        }
        let reach: HashSet<seg::Oid> = order.iter().filter_map(|id| store::oid_of(id)).collect();
        // Incremental when the set is still part of this history: the new commits become one
        // more segment. Otherwise (first build, rewritten history) everything is read again.
        let extend = previous
            .as_ref()
            .filter(|_| !held.is_empty() && held.iter().all(|o| reach.contains(o)));
        let ids: Vec<String> =
            if extend.is_some() { todo.into_iter().cloned().collect() } else { order.clone() };
        let total = ids.len();
        progress(Progress { done: 0, total });
        let mut read = 0_usize;
        // The stream's wall time holds the git reads and the fold; the fold is timed apart.
        let mut fold = std::time::Duration::ZERO;
        let mut merge = None;
        ph.start("stream");
        if let Some(prev) = extend {
            // Only the new commits are held, never the history.
            let mut fresh = Vec::with_capacity(ids.len());
            source.stream_records(
                &ids,
                BATCH,
                &mut |r| {
                    self.go_on()?;
                    read += 1;
                    fresh.push(r);
                    Ok(())
                },
                &mut |done| progress(Progress { done, total }),
            )?;
            ph.start("append");
            self.go_on()?;
            if !fresh.is_empty() {
                let after = store::append_segment(&self.dir, prev.clone(), &shallow, &fresh)
                    .map_err(seg)?;
                let segment = seg::format::Kind::Segment as u8;
                if after.live.iter().filter(|f| f.kind == segment).count() > store::MAX_SEGMENTS {
                    merge = Some((order, shallow));
                }
            }
        } else {
            // Each record folds into the segment as it passes: the build never holds history.
            let mut build =
                store::SegmentBuild::new(&self.dir, previous.as_ref(), &shallow).map_err(seg)?;
            let streamed = source.stream_records(
                &ids,
                BATCH,
                &mut |r| {
                    self.go_on()?;
                    read += 1;
                    let t = std::time::Instant::now();
                    let pushed = build.push(&r).map_err(seg);
                    fold += t.elapsed();
                    pushed
                },
                &mut |done| progress(Progress { done, total }),
            );
            if let Err(e) = streamed.and_then(|()| self.go_on()) {
                build.discard();
                return Err(e);
            }
            ph.start("finish");
            build.finish(previous, Vec::new()).map_err(seg)?;
        }
        ph.add("stream.fold", fold);
        self.sync_side(source)?;
        // A skipped commit is not in the set, so it stays pending and the next update appends
        // it; until then HEAD's pages read the reach view, and the tip marker is not written.
        self.go_on()?;
        if read < total {
            skipped(total - read, &self.dir);
        } else if let Some(t) = tip {
            cache::write_atomic(&self.dir, TIP_FILE, t.as_bytes())?;
        }
        Ok(merge)
    }

    /// Brings the message column and the tip file in step with the set, each step
    /// skipped once halted.
    fn sync_side(&self, source: &dyn CommitSource) -> io::Result<()> {
        let ph = &self.phases;
        self.go_on()?;
        ph.start("messages");
        self.sync_messages(source)?;
        self.go_on()?;
        ph.start("tip");
        tip::sync(&self.dir)?;
        ph.end();
        Ok(())
    }

    /// Folds the segments into one once there are more than 8, off the caller's thread, so a
    /// request never waits on it. Records come from the set itself, raw as stored. The merge
    /// takes the writer lock itself.
    fn merge_in_background(&self, order: Vec<String>, shallow: Vec<String>) {
        let dir = self.dir.clone();
        let run = move || store::merge_stored(&dir, &order, &shallow);
        if self.merge_inline.load(std::sync::atomic::Ordering::Relaxed) {
            return run()
                .map_or_else(|e| crate::warn(format_args!("segment merge failed: {e}")), drop);
        }
        let spawned =
            std::thread::Builder::new().name("gitterrace-merge".into()).spawn(move || {
                if let Err(e) = run() {
                    crate::warn(format_args!("segment merge failed: {e}"));
                }
            });
        if let Err(e) = spawned {
            crate::warn(format_args!("segment merge not started: {e}"));
        }
    }
}

#[cfg(test)]
#[path = "halt_tests.rs"]
mod tests;
