//! The insights source over the segment set, and its release when idle.

use std::{
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use crate::app::App;
use crate::error::Result;

impl App {
    /// What every insights page reads for `head`: the segment set, or the reach view over it
    /// for another commit. `None` until the first build finished (the indexing page) and for a
    /// commit too far from the set (the reindex page). Kept for one (commit, generation), so a
    /// HEAD move or a rebuild never serves a stale source.
    pub(crate) fn source(&self, head: &str) -> Result<Option<crate::insights::Src>> {
        self.check_unreadable()?;
        let key = (head.to_owned(), self.index.generation());
        if let Ok(g) = self.caches.source.lock()
            && let Some((k, s)) = g.as_ref()
            && *k == key
        {
            self.touch();
            return Ok(Some(Arc::clone(s)));
        }
        if !self.ready() {
            self.demand();
            return Ok(None);
        }
        // HEAD (the rev the set was last built for) reads the segment files and holds no
        // history. Another commit reads the set plus what it lacks, read from git.
        if self.index.newer_format().is_none()
            && let Some(set) = self.index.disk_source(&self.repo, head)?
        {
            self.touch();
            let checked = Arc::clone(&set);
            if let Ok(mut d) = self.caches.disk.lock() {
                *d = Some(Arc::new(move || gitterrace_index::unreadable(&checked)));
            }
            let src: crate::insights::Src = set;
            if let Ok(mut g) = self.caches.source.lock() {
                *g = Some((key, Arc::clone(&src)));
            }
            return Ok(Some(src));
        }
        if self.index.newer_format().is_none()
            && let Some(view) = self.index.rev_source(&self.repo, head, head)?
        {
            match view {
                gitterrace_index::feed::RevView::Ready(reach) => {
                    let checked = Arc::clone(&reach);
                    if let Ok(mut d) = self.caches.disk.lock() {
                        *d = Some(Arc::new(move || {
                            gitterrace_index::unreadable(checked.base())
                        }));
                    }
                    let src: crate::insights::Src = reach;
                    if let Ok(mut g) = self.caches.source.lock() {
                        *g = Some((key, Arc::clone(&src)));
                    }
                    return Ok(Some(src));
                }
                // Too far from the set: no records load, no reindex, the page says so.
                gitterrace_index::feed::RevView::NeedsReindex { read } => {
                    if let Ok(mut f) = self.caches.far.lock() {
                        *f = Some((key, read));
                    }
                    return Ok(None);
                }
            }
        }
        // Ready but no set holds or reaches it: a newer build owns the cache, or the set is
        // being rebuilt. The indexing page, never data from anywhere else.
        Ok(None)
    }

    /// A block that failed its check while serving deletes the cache, drops
    /// every source over it and rebuilds. Until the rebuild is done pages show the indexing
    /// page, never data from a corrupt set.
    fn check_unreadable(&self) -> Result<()> {
        let bad = self
            .caches
            .disk
            .lock()
            .ok()
            .and_then(|d| d.as_ref().map(Arc::clone))
            .and_then(|check| check());
        let Some(why) = bad else {
            return Ok(());
        };
        tracing::warn!("cache unreadable while serving, rebuilding: {why}");
        if let Ok(mut d) = self.caches.disk.lock() {
            d.take();
        }
        if let Ok(mut g) = self.caches.source.lock() {
            g.take();
        }
        self.index.rebuild()?;
        self.ready.store(false, Ordering::Release);
        // HEAD did not move, so the loop would not rebuild it on its own.
        self.rebuild.store(true, Ordering::Release);
        self.wake.notify_one();
        self.demand();
        Ok(())
    }

    /// Commits read before `head` was found too far from the segment set, when its insights
    /// are not served for that reason (the last [`App::source`] call for it said so).
    pub(crate) fn far(&self, head: &str) -> Option<usize> {
        let key = (head.to_owned(), self.index.generation());
        let f = self.caches.far.lock().ok()?;
        f.as_ref().filter(|(k, _)| *k == key).map(|(_, n)| *n)
    }

    #[cfg(test)]
    pub(crate) fn source_held(&self) -> bool {
        self.caches.source.lock().is_ok_and(|g| g.is_some())
    }

    /// Idle releases that freed something so far.
    pub(crate) fn releases(&self) -> u64 {
        self.releases.load(Ordering::Relaxed)
    }

    /// Marks the caches used by a visitor. Warm-up reads do not count: a server that only
    /// warmed itself is idle, and its caches go one idle period after the warm-up started.
    fn touch(&self) {
        if self.warming.load(Ordering::Relaxed) > 0 {
            return;
        }
        if let Ok(mut g) = self.records_used.lock() {
            *g = Instant::now();
        }
    }

    /// Marks warm-up work until the guard drops: reads meanwhile do not delay
    /// [`App::release_idle`]. A visitor's page during a warm-up is served as always, only the
    /// release may come one period sooner and the next page builds again.
    pub(crate) fn warming(self: &Arc<Self>) -> Warming {
        self.warming.fetch_add(1, Ordering::Relaxed);
        Warming(Arc::clone(self))
    }

    /// Drops the source and every per-request cache once no page read them for `idle`.
    /// Returns whether it released anything. A page in flight keeps its own `Arc`, so it is
    /// never cut short; the next page builds again. Only the rendered-page memo stays.
    pub(crate) fn release_idle(&self, idle: Duration) -> bool {
        let quiet = self.records_used.lock().is_ok_and(|g| g.elapsed() >= idle);
        if !quiet {
            return false;
        }
        let released = self.caches.clear_all();
        gitterrace_index::Insights::release_shared(&format!("{}\u{0}", self.base));
        // The API opens its own source; a caller there is a client that can wait a second too.
        if let Some(api) = self.api.get() {
            api.release_records();
        }
        if released {
            self.releases.fetch_add(1, Ordering::Relaxed);
        }
        released
    }
}

/// Warm-up in progress, see [`App::warming`].
pub(crate) struct Warming(Arc<App>);

impl Drop for Warming {
    fn drop(&mut self) {
        self.0.warming.fetch_sub(1, Ordering::Relaxed);
    }
}
