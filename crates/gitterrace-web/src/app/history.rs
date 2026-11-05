//! Caches keyed by commit: file lists, path logs, head lines, ignore rules, totals.

use std::sync::Arc;

use crate::app::{App, MAX_IGNORE_BYTES};
use crate::error::Result;

impl App {
    /// Paths of all files at a full commit id. Commits are immutable, so entries never expire.
    pub(crate) fn file_paths(&self, commit: &str) -> Result<Arc<Vec<String>>> {
        if let Ok(g) = self.caches.file_lists.lock()
            && let Some(v) = g.get(commit)
        {
            return Ok(v);
        }
        let list: Vec<String> = gitterrace_git::files(&self.repo, commit)?
            .into_iter()
            .map(|f| f.path)
            .collect();
        let list = Arc::new(list);
        if let Ok(mut g) = self.caches.file_lists.lock() {
            g.put(commit.to_owned(), Arc::clone(&list));
        }
        Ok(list)
    }

    /// The commits `git log <commit> -- path` lists, newest first, or `None` while the index is
    /// not ready for `commit` (the caller walks then). Built once per
    /// (commit, path, index generation) from the records, so a repeat costs a map lookup.
    pub(crate) fn path_log(&self, commit: &str, path: &str) -> Result<Option<Arc<PathLog>>> {
        let Some(src) = self.source(commit)? else {
            return Ok(None);
        };
        let key = format!("{commit}\0{path}\0{}", self.index.generation());
        if let Ok(g) = self.caches.path_logs.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(Some(v));
        }
        let owned = src.run(|s| touching_history(&**s, path))?;
        let history: Vec<(&str, &[String], bool)> = owned
            .iter()
            .map(|(id, parents, t)| (id.as_str(), parents.as_slice(), *t))
            .collect();
        let (ids, complete) = self.repo.touching(path, &history);
        let ids = Arc::new(PathLog { ids, complete });
        if let Ok(mut g) = self.caches.path_logs.lock() {
            g.put(key, Arc::clone(&ids));
        }
        Ok(Some(ids))
    }

    /// The languages skeleton of `head`, built once per head and index generation.
    pub(crate) fn chain(
        &self,
        head: &str,
        src: &dyn gitterrace_index::feed::InsightsSource,
        filter: &gitterrace_index::PathFilter,
    ) -> Arc<crate::languages::Skeleton> {
        let key = (head.to_owned(), self.index.generation());
        if let Ok(g) = self.caches.chain.lock()
            && let Some((k, s)) = g.as_ref()
            && *k == key
        {
            return Arc::clone(s);
        }
        let s = Arc::new(crate::languages::Skeleton::build(src, filter));
        if let Ok(mut g) = self.caches.chain.lock() {
            *g = Some((key, Arc::clone(&s)));
        }
        s
    }

    /// Line count of every file in `head`, built once per head. Reading the line cache,
    /// walking the tree and saving the cache cost tens of ms per insights page otherwise.
    pub(crate) fn head_lines(&self, head: &str) -> Arc<Vec<gitterrace_index::HeadFile>> {
        if let Ok(g) = self.caches.head_lines.lock()
            && let Some((id, f)) = g.as_ref()
            && id == head
        {
            return Arc::clone(f);
        }
        let mut cache = gitterrace_index::LineCache::load(self.index.dir());
        let files = Arc::new(
            gitterrace_index::head_files(&self.repo, head, &mut cache).unwrap_or_default(),
        );
        if let Err(e) = cache.save(self.index.dir()) {
            tracing::warn!("line cache not saved: {e}");
        }
        if let Ok(mut g) = self.caches.head_lines.lock() {
            *g = Some((head.to_owned(), Arc::clone(&files)));
        }
        files
    }

    /// The ignore rules committed at `commit`. The files are found once per commit; the walk
    /// that finds them costs as much as everything else an insights page does.
    pub(crate) fn ignore_set(&self, commit: &str) -> Option<gitterrace_git::IgnoreSet> {
        if let Ok(g) = self.caches.ignores.lock()
            && let Some((id, found)) = g.as_ref()
            && id == commit
        {
            return Some(gitterrace_git::IgnoreSet::from_files(found.iter().cloned()));
        }
        let paths =
            gitterrace_git::files_named(&self.repo, commit, &[".gitignore", ".ignore"]).ok()?;
        let mut found: Vec<(String, String)> = paths
            .iter()
            .filter_map(|p| {
                let bytes = self.repo.blob_bytes(commit, p).ok()?;
                (bytes.len() <= MAX_IGNORE_BYTES)
                    .then(|| (p.clone(), String::from_utf8_lossy(&bytes).into_owned()))
            })
            .collect();
        // Shallow first: the search tries the last added list first, so deeper rules win.
        found.sort_by_key(|(path, _)| path.matches('/').count());
        let found = Arc::new(found);
        if let Ok(mut g) = self.caches.ignores.lock() {
            *g = Some((commit.to_owned(), Arc::clone(&found)));
        }
        Some(gitterrace_git::IgnoreSet::from_files(found.iter().cloned()))
    }

    /// All-history totals of `commit` with `opts`, computed once per index generation.
    pub(crate) fn totals(
        &self,
        commit: &str,
        src: &dyn gitterrace_index::feed::InsightsSource,
        opts: gitterrace_index::Options,
    ) -> gitterrace_index::PeriodStats {
        let key = format!("{commit}\0{}\0{opts:?}", self.index.generation());
        if let Ok(g) = self.caches.totals.lock()
            && let Some((k, t)) = g.as_ref()
            && *k == key
        {
            return t.clone();
        }
        let ignore = opts
            .respect_ignore
            .then(|| self.ignore_set(commit))
            .flatten();
        let t = gitterrace_index::Insights::from_source(src, opts, ignore)
            .period_summary(gitterrace_index::Period::All)
            .current;
        if let Ok(mut g) = self.caches.totals.lock() {
            *g = Some((key, t.clone()));
        }
        t
    }
}

/// A path's log from the index: `ids` newest first. When not `complete`, an unreadable commit
/// cut it short and only the pages inside `ids` may be served from it.
pub(crate) struct PathLog {
    pub(crate) ids: Vec<String>,
    pub(crate) complete: bool,
}

impl crate::bounded::Weight for PathLog {
    fn weight(&self) -> usize {
        self.ids.weight()
    }
}

/// Every commit of the records, newest first (records are oldest first, in the walk's order
/// reversed), with its parents and whether its first-parent diff touches a path at or under
/// `path`. A commit that does not holds `path` as its first parent does.
fn touching_history(
    src: &dyn gitterrace_index::feed::InsightsSource,
    path: &str,
) -> Vec<(String, Vec<String>, bool)> {
    use gitterrace_index::feed::CommitRange;
    let ranks: Vec<_> = src.commits_in(CommitRange::All).map(|c| c.rank).collect();
    let mut out: Vec<(String, Vec<String>, bool)> = src
        .rows_for(ranks)
        .map(|(rank, rows)| {
            let touched = path.is_empty()
                || rows.iter().any(|f| {
                    src.path(f.path)
                        .is_some_and(|p| crate::filter::under(&p, path))
                        || f.old
                            .and_then(|o| src.path(o))
                            .is_some_and(|o| crate::filter::under(&o, path))
                });
            let id = src
                .commit_id(rank)
                .as_deref()
                .unwrap_or_default()
                .to_owned();
            let parents = src
                .parent_ids(rank)
                .iter()
                .map(ToString::to_string)
                .collect();
            (id, parents, touched)
        })
        .collect();
    out.reverse();
    out
}
