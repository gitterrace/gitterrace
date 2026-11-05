//! What an answer does when the insights cache fails a read: start the rebuild (the
//! server's, or one of its own), answer 503 meanwhile, and never write a newer build's cache.

use std::sync::Arc;

use gitterrace_index::feed::InsightsSource;

use crate::core::{ApiState, Rebuild};
use crate::error::ApiError;

impl ApiState {
    /// Builds `f` over the source of `rev`. A storage error, whether met opening the set or
    /// while `f` reads it, starts the rebuild and answers 503: never a 500, never an answer
    /// built from a failed read.
    pub(crate) fn read<T>(
        &self,
        rev: &str,
        f: impl FnOnce(&Arc<dyn InsightsSource>) -> T,
    ) -> Result<T, ApiError> {
        let src = match self.records(rev) {
            Err(ApiError::Unreadable(e)) => return Err(self.unreadable(e)),
            other => other?,
        };
        src.run(f).map_err(|e| self.unreadable(e))
    }

    /// Hands an unreadable cache to `rebuild` instead of rebuilding it here. The web server
    /// passes its own, so a page and an API call that meet one bad block start one rebuild.
    pub fn on_unreadable(&self, rebuild: Rebuild) {
        if let Ok(mut g) = self.on_unreadable.write() {
            *g = Some(rebuild);
        }
    }

    /// The cache failed a read: drop what was read from it and start the rebuild. A cache a
    /// newer build owns is never written; it stays as it is and the answer says so.
    fn unreadable(&self, e: gitterrace_index::feed::SourceError) -> ApiError {
        self.release_records();
        if let Some(found) = self.index.newer_owner() {
            return ApiError::NewerCache(found);
        }
        let hook = self.on_unreadable.read().ok().and_then(|g| g.clone());
        match hook {
            Some(rebuild) => rebuild(),
            None => self.rebuild_alone(),
        }
        ApiError::Unreadable(e)
    }

    /// The rebuild of a state without a server (the MCP server): delete the cache, then
    /// build it again on a thread of its own, once at a time.
    #[expect(
        clippy::print_stderr,
        reason = "the MCP server has no log: a rebuild failure goes to the user's stderr"
    )]
    fn rebuild_alone(&self) {
        use std::sync::atomic::Ordering;
        if self.rebuilding.swap(true, Ordering::AcqRel) {
            return;
        }
        let state = self.clone();
        let run = move || {
            let r = state.index.rebuild().map_err(ApiError::from);
            if let Err(e) = r.and_then(|()| state.refresh_index()) {
                eprintln!("gitterrace: rebuilding the unreadable cache failed: {e}");
            }
            state.rebuilding.store(false, Ordering::Release);
        };
        if std::thread::Builder::new()
            .name("gitterrace-rebuild".into())
            .spawn(run)
            .is_err()
        {
            self.rebuilding.store(false, Ordering::Release);
        }
    }
}
