//! The rendered-page memo and its keys.

use std::{
    io::{Read, Write},
    sync::Arc,
    time::Instant,
};

use crate::app::{App, MEMO_TTL};

/// A rendered page held deflated. The memo is the one cache an idle release keeps, and pages
/// of tens of KB fall into multi-page slab classes on 64 KB page kernels, where a few kept
/// pages pinned whole slabs: 15 MB resident after the release on aarch64 Linux against a
/// 12 MB cap. Deflated, the same pages weigh a tenth and pin a tenth of the slabs.
pub(crate) struct Packed(Box<[u8]>);

impl Packed {
    /// `None` only when the encoder fails, which in memory it does not: the page is not kept.
    fn pack(html: &str) -> Option<Self> {
        let mut z = flate2::write::DeflateEncoder::new(
            Vec::with_capacity(html.len() / 8),
            flate2::Compression::fast(),
        );
        z.write_all(html.as_bytes()).ok()?;
        Some(Self(z.finish().ok()?.into_boxed_slice()))
    }

    fn unpack(&self) -> Option<String> {
        let mut out = String::new();
        flate2::read::DeflateDecoder::new(&self.0[..]).read_to_string(&mut out).ok()?;
        Some(out)
    }
}

impl crate::bounded::Weight for Packed {
    fn weight(&self) -> usize {
        self.0.len()
    }
}

impl App {
    /// A rendered page for `key`, if fresh, carrying the live version of now. The key does not
    /// move with the version (it moves on every worktree change, which would empty the memo),
    /// so the page's `data-live-v` is stamped here: a stale one makes the tab hear everything.
    pub(crate) fn memo_get(&self, key: &str) -> Option<String> {
        let (at, packed) = self.memo.lock().ok()?.get(key)?;
        if at.elapsed() >= MEMO_TTL {
            return None;
        }
        Some(stamp(&packed.unpack()?, self.version()))
    }

    /// The lock held while `key` is rendered. A second request for the same cold key waits on
    /// it and then finds the first one's page in the memo, so two concurrent renders of one
    /// page (the warm-up and the first visitor, right after the index is ready) become one.
    /// The entry goes when the last holder drops it.
    pub(crate) fn render_lock(&self, key: &str) -> RenderLock<'_> {
        let lock = self.rendering.lock().map_or_else(
            |_| Arc::new(tokio::sync::Mutex::new(())),
            |mut g| Arc::clone(g.entry(key.to_owned()).or_default()),
        );
        RenderLock { app: self, key: key.to_owned(), lock: Some(lock) }
    }

    pub(crate) fn memo_put(&self, key: String, html: &str) {
        let Some(packed) = Packed::pack(html) else {
            return;
        };
        if let Ok(mut g) = self.memo.lock() {
            g.put(key, (Instant::now(), Arc::new(packed)));
        }
    }

    /// Drops every rendered page.
    pub(crate) fn clear_memo(&self) {
        if let Ok(mut g) = self.memo.lock() {
            g.clear();
        }
    }

    /// The query reduced to what the stats pages read, sorted: unknown parameters and the
    /// order of known ones must not make a new memo entry.
    pub(crate) fn canonical_query(raw: Option<&str>) -> String {
        const KNOWN: [&str; 15] = [
            "q",
            "period",
            "page",
            "bots",
            "merges",
            "exclude",
            "generated",
            "ignore",
            "ws",
            "base",
            "all",
            "metric",
            "from",
            "to",
            "mode",
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

    /// Key for a rendered page: moves with HEAD, the selected ref and its commit, the index
    /// file, the branch name, the dirty chip and the query, so any of them changing yields a
    /// fresh render. The ref's name counts on its own: links carry it, so `?rev=dev` must not
    /// get the `HEAD` page when both name one commit.
    pub(crate) fn memo_key(&self, tab: &str, query: Option<&str>) -> String {
        let rev = crate::fmt::selected();
        format!(
            "{tab}\u{0}{}\u{0}{}\u{0}{rev}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
            self.base,
            self.repo.resolve("HEAD").unwrap_or_default(),
            self.repo.resolve(&rev).unwrap_or_default(),
            self.index.generation(),
            self.repo.head_branch().unwrap_or_default(),
            self.dirty(),
            query.unwrap_or_default()
        )
    }
}

/// `html` with the value of its first `data-live-v` set to `v`. A page without one is as is.
fn stamp(html: &str, v: u64) -> String {
    const KEY: &str = "data-live-v=\"";
    let Some(start) = html.find(KEY).map(|at| at + KEY.len()) else {
        return html.to_owned();
    };
    let Some(end) = html[start..].find('"').map(|e| start + e) else {
        return html.to_owned();
    };
    format!("{}{v}{}", &html[..start], &html[end..])
}

#[cfg(test)]
#[path = "memo_tests.rs"]
mod tests;

/// A claim on rendering one memo key, from [`App::render_lock`].
pub(crate) struct RenderLock<'a> {
    app: &'a App,
    key: String,
    lock: Option<Arc<tokio::sync::Mutex<()>>>,
}

impl RenderLock<'_> {
    /// Waits until no other request renders this key.
    pub(crate) async fn hold(&self) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        Some(self.lock.as_ref()?.lock().await)
    }
}

impl Drop for RenderLock<'_> {
    fn drop(&mut self) {
        let lock = self.lock.take();
        if let Ok(mut g) = self.app.rendering.lock() {
            // Ours plus the table's: nobody else waits, so the entry goes.
            if lock.as_ref().is_some_and(|l| Arc::strong_count(l) <= 2) {
                g.remove(&self.key);
            }
        }
    }
}
