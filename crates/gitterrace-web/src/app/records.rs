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
    pub(crate) fn source(&self, head: &str) -> Result<Option<crate::insights::CheckedSrc>> {
        match self.open_source(head) {
            // The set failed a read while opening: the same rebuild as one met while serving.
            Err(crate::error::Error::Index(e))
                if gitterrace_index::feed::SourceError::of(&e).is_some() =>
            {
                tracing::warn!("cache unreadable on open, rebuilding: {e}");
                self.unreadable_now()?;
                Ok(None)
            }
            other => other,
        }
    }

    fn open_source(&self, head: &str) -> Result<Option<crate::insights::CheckedSrc>> {
        self.check_unreadable()?;
        // Ready first: the warm-up keeps HEAD's source before the index is marked ready, and a
        // page served from it then would say ready while every other revision still gets the
        // indexing page.
        if !self.ready() {
            self.demand();
            return Ok(None);
        }
        let key = (head.to_owned(), self.index.generation());
        if let Ok(g) = self.caches.source.lock()
            && let Some((k, s)) = g.as_ref()
            && *k == key
        {
            self.touch();
            return Ok(Some(s.share()));
        }
        self.open_set(key, head)
    }

    /// Opens and keeps the set of `head` without the ready check: the refresh builds the
    /// tables the per-file pages share over it just before it marks the index ready.
    pub(crate) fn source_before_ready(
        &self,
        head: &str,
    ) -> Result<Option<crate::insights::CheckedSrc>> {
        self.check_unreadable()?;
        let key = (head.to_owned(), self.index.generation());
        if let Ok(g) = self.caches.source.lock()
            && let Some((k, s)) = g.as_ref()
            && *k == key
        {
            return Ok(Some(s.share()));
        }
        self.open_set(key, head)
    }

    fn open_set(
        &self,
        key: (String, u64),
        head: &str,
    ) -> Result<Option<crate::insights::CheckedSrc>> {
        // HEAD (the rev the set was last built for) reads the segment files and holds no
        // history. Another commit reads the set plus what it lacks, read from git.
        if self.index.newer_format().is_none() {
            match self.index.source_for(&self.repo, head, head)? {
                Some(gitterrace_index::Selected::Ready(src)) => {
                    self.touch();
                    return Ok(Some(self.keep(key, src)));
                }
                // Too far from the set: no records load, no reindex, the page says so.
                Some(gitterrace_index::Selected::NeedsReindex { read }) => {
                    if let Ok(mut f) = self.caches.far.lock() {
                        *f = Some((key, read));
                    }
                    return Ok(None);
                }
                None => {}
            }
        }
        // Ready but no set holds or reaches it: a newer build owns the cache, or the set is
        // being rebuilt. The indexing page, never data from anywhere else.
        Ok(None)
    }

    /// Keeps `src` as the source of `key`, the one every page of that commit shares.
    fn keep(
        &self,
        key: (String, u64),
        src: crate::insights::CheckedSrc,
    ) -> crate::insights::CheckedSrc {
        if let Ok(mut g) = self.caches.source.lock() {
            *g = Some((key, src.share()));
        }
        src
    }

    /// A block that failed its check while serving deletes the cache, drops
    /// every source over it and rebuilds. Until the rebuild is done pages show the indexing
    /// page, never data from a corrupt set. The page that met the error was refused by
    /// `Checked::run` already; this runs at the next request.
    fn check_unreadable(&self) -> Result<()> {
        let bad = self
            .caches
            .source
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|(_, s)| s.failure()));
        let Some(why) = bad else {
            return Ok(());
        };
        tracing::warn!("cache unreadable while serving, rebuilding: {why}");
        self.unreadable_now()
    }

    /// `f` over the source of `head`, for a page that only uses the index to go faster and
    /// walks git without it: `None` while there is no source, and when a read fails (the cache
    /// is rebuilt then, and the page walks git as if it had none).
    pub(crate) fn accelerate<T>(
        &self,
        head: &str,
        f: impl FnOnce(&crate::insights::Src) -> T,
    ) -> Result<Option<T>> {
        let Some(src) = self.source(head)? else {
            return Ok(None);
        };
        match src.run(f) {
            Ok(v) => Ok(Some(v)),
            Err(e) => self.unreadable(&e).map(|()| None),
        }
    }

    /// A page met `e` while it was built: the cache goes, the visitor gets the indexing page.
    pub(crate) fn unreadable(&self, e: &gitterrace_index::feed::SourceError) -> Result<()> {
        tracing::warn!("cache unreadable while a page was built, rebuilding: {e}");
        self.unreadable_now()
    }

    /// Drops everything read from the cache, deletes it and starts the rebuild.
    pub(crate) fn unreadable_now(&self) -> Result<()> {
        // Everything built over the source goes with it: a rebuild of the same history gets
        // the same generation, so a table built while a read failed would be served again.
        self.caches.clear_all();
        gitterrace_index::Insights::release_shared(&format!("{}\u{0}", self.base));
        // The page memo too: a sidebar or count rendered while a read failed would otherwise
        // be served from the rebuilt cache, whose generation is the same.
        self.clear_memo();
        self.index.rebuild()?;
        self.ready.store(false, Ordering::Release);
        // HEAD did not move, so the loop would not rebuild it on its own.
        self.rebuild.store(true, Ordering::Release);
        self.wake.notify_one();
        self.demand();
        Ok(())
    }

    /// Commits read before `head` was found too far from the segment set, when its insights
    /// are not served for that reason. The answer kept by the last [`App::source`] call can
    /// be gone by the time a page asks: a far page is no use, so the idle release may clear
    /// it in between, and a background segment merge moves the generation. Then the index is
    /// asked again, which reads at most the fresh-commit cap and loads no records.
    pub(crate) fn far(&self, head: &str) -> Option<usize> {
        let key = (head.to_owned(), self.index.generation());
        if let Ok(f) = self.caches.far.lock()
            && let Some((k, n)) = f.as_ref()
            && *k == key
        {
            return Some(*n);
        }
        if !self.ready() || self.index.newer_format().is_some() {
            return None;
        }
        match self.index.source_for(&self.repo, head, head) {
            Ok(Some(gitterrace_index::Selected::NeedsReindex { read })) => {
                if let Ok(mut f) = self.caches.far.lock() {
                    *f = Some((key, read));
                }
                Some(read)
            }
            _ => None,
        }
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

    /// Drops the source, every per-request cache, the blame cache and the highlighted-code memo
    /// once no page read them for `idle`.
    /// Returns whether it released anything. A page in flight keeps its own `Arc`, so it is
    /// never cut short; the next page builds again. Only the rendered-page memo stays.
    pub(crate) fn release_idle(&self, idle: Duration) -> bool {
        let quiet = self.records_used.lock().is_ok_and(|g| g.elapsed() >= idle);
        if !quiet {
            return false;
        }
        // Bitwise or: every cache empties, even after one reported that it held something.
        let released = self.caches.clear_all()
            | self.repo.clear_blame_cache()
            | crate::views::clear_code_memo();
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

#[cfg(test)]
#[path = "far_tests.rs"]
mod tests;
