//! The rendered-page memo and its keys.

use std::{sync::Arc, time::Instant};

use crate::app::{App, MEMO_TTL};

impl App {
    /// A rendered page for `key`, if fresh.
    pub(crate) fn memo_get(&self, key: &str) -> Option<Arc<String>> {
        let (at, html) = self.memo.lock().ok()?.get(key)?;
        (at.elapsed() < MEMO_TTL).then_some(html)
    }

    pub(crate) fn memo_put(&self, key: String, html: Arc<String>) {
        if let Ok(mut g) = self.memo.lock() {
            g.put(key, (Instant::now(), html));
        }
    }

    /// The query reduced to what the stats pages read, sorted: unknown parameters and the
    /// order of known ones must not make a new memo entry.
    pub(crate) fn canonical_query(raw: Option<&str>) -> String {
        const KNOWN: [&str; 14] = [
            "type",
            "scope",
            "author",
            "path",
            "period",
            "page",
            "bots",
            "exclude",
            "generated",
            "ignore",
            "ws",
            "base",
            "all",
            "metric",
        ];
        let mut pairs: Vec<&str> = raw
            .unwrap_or_default()
            .split('&')
            .filter(|p| {
                let key = p.split_once('=').map_or(*p, |(k, _)| k);
                KNOWN.contains(&key) || key == "head"
            })
            .collect();
        pairs.sort_unstable();
        pairs.join("&")
    }

    /// Key for a rendered page: moves with HEAD, the selected ref's commit, the index file, the
    /// branch name, the dirty chip and the query, so any of them changing yields a fresh render.
    pub(crate) fn memo_key(&self, tab: &str, query: Option<&str>) -> String {
        format!(
            "{tab}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
            self.base,
            self.repo.resolve("HEAD").unwrap_or_default(),
            self.repo
                .resolve(&crate::fmt::selected())
                .unwrap_or_default(),
            self.index.generation(),
            self.repo.head_branch().unwrap_or_default(),
            self.dirty(),
            query.unwrap_or_default()
        )
    }
}
