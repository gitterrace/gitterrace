//! The web pages' share with the API: their source of a tip and the derived tables built over
//! it, so an API answer over the tip the pages read builds no second copy.

use std::sync::Arc;

use gitterrace_index::feed::{Checked, InsightsSource};
use gitterrace_index::{Insights, Options};

use crate::core::ApiState;
use crate::error::ApiError;

/// What [`Pages::source`] calls.
pub type PageSource =
    Arc<dyn Fn(&str) -> Option<(Checked<dyn InsightsSource>, String)> + Send + Sync>;

/// What the web pages share with the API, see [`ApiState::share_pages`].
#[derive(Clone)]
pub struct Pages {
    /// The tables the pages slice.
    pub tables: Arc<gitterrace_index::SharedTables>,
    /// The pages' source of a resolved tip and the key their tables hold it under, `None`
    /// while the pages hold no source for it.
    pub source: PageSource,
}

impl ApiState {
    /// Statistics over `src` under `opts`, with the ignore rules of `rev` when asked: what
    /// the web pages build for the same options.
    pub(super) fn insights<'a>(
        &self,
        rev: &str,
        src: &'a Arc<dyn InsightsSource>,
        opts: Options,
    ) -> Insights<'a> {
        let respect = opts.respect_ignore;
        let ignore = || respect.then(|| self.ignore_set(rev)).flatten();
        if let Ok(tip) = self.repo.resolve(rev)
            && let Some((pages, key)) = self.pages_source(&tip)
            && pages.holds(src)
            && let Some(tables) = self.pages_tables()
        {
            return Insights::shared_source(&tables, &**src, opts, &key, ignore);
        }
        let key = format!("{rev}\u{0}{}", self.index.generation());
        Insights::shared_source(&self.tables, &**src, opts, &key, ignore)
    }

    /// Lets the API read the web pages' source and slice their tables, see [`Pages`].
    pub fn share_pages(&self, pages: Pages) {
        if let Ok(mut g) = self.pages.write() {
            *g = Some(pages);
        }
    }

    pub(super) fn pages_source(&self, tip: &str) -> Option<(Checked<dyn InsightsSource>, String)> {
        let source = self.pages.read().ok()?.as_ref()?.source.clone();
        source(tip)
    }

    pub(super) fn pages_tables(&self) -> Option<Arc<gitterrace_index::SharedTables>> {
        Some(Arc::clone(&self.pages.read().ok()?.as_ref()?.tables))
    }

    /// The source an insights answer over `rev` reads, for the server's tests that count what
    /// the API and the pages build over it.
    #[doc(hidden)]
    pub fn insights_source(&self, rev: &str) -> Result<Checked<dyn InsightsSource>, ApiError> {
        self.records(rev)
    }
}
