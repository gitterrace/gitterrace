//! Caches keyed by commit: file lists, path logs, head lines, ignore rules, totals.

use std::sync::Arc;

use crate::app::App;
use crate::error::Result;

impl App {
    /// Paths of all files at a full commit id. Commits are immutable, so entries never expire.
    pub(crate) fn file_paths(&self, commit: &str) -> Result<Arc<Vec<String>>> {
        if let Ok(g) = self.caches.file_lists.lock()
            && let Some(v) = g.get(commit)
        {
            return Ok(v);
        }
        let mut list: Vec<String> =
            gitterrace_git::files(&self.repo, commit)?.into_iter().map(|f| f.path).collect();
        // Without a ready index the list stays alphabetical and is not kept, so the next call
        // looks again once the records exist.
        let ranked = self.accelerate(commit, |s| last_touched(&**s))?;
        if let Some(touched) = &ranked {
            // Stable: files never touched, or tied, keep their alphabetical order.
            list.sort_by_key(|p| std::cmp::Reverse(touched.get(p).copied()));
        }
        let list = Arc::new(list);
        if ranked.is_some()
            && let Ok(mut g) = self.caches.file_lists.lock()
        {
            g.put(commit.to_owned(), Arc::clone(&list));
        }
        Ok(list)
    }

    /// The commits `git log <commit> -- path` lists, newest first, or `None` while the index is
    /// not ready for `commit` (the caller walks then). Built once per
    /// (commit, path, index generation) from the records, so a repeat costs a map lookup.
    pub(crate) fn path_log(&self, commit: &str, path: &str) -> Result<Option<Arc<PathLog>>> {
        let key = format!("{commit}\0{path}\0{}", self.index.generation());
        if self.source(commit)?.is_some()
            && let Ok(g) = self.caches.path_logs.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(Some(v));
        }
        let Some(owned) = self.accelerate(commit, |s| touching_history(&**s, path))? else {
            return Ok(None);
        };
        let history: Vec<(&str, &[String], bool)> =
            owned.iter().map(|(id, parents, t)| (id.as_str(), parents.as_slice(), *t)).collect();
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
        self.chain_walks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let s = Arc::new(self.build_chain(head, src, filter));
        // Built over a read that failed: the caller refuses it, and a rebuild keeps the
        // generation, so a kept copy would be served over the clean cache.
        if src.failure().is_none()
            && let Ok(mut g) = self.caches.chain.lock()
        {
            *g = Some((key, Arc::clone(&s)));
        }
        s
    }

    /// The skeleton from the state an earlier build saved, reading only the commits since,
    /// else from nothing; the state of this build is saved for the next one. A state that is
    /// missing, damaged or for other rules is no state, so the result is the same either way.
    fn build_chain(
        &self,
        head: &str,
        src: &dyn gitterrace_index::feed::InsightsSource,
        filter: &gitterrace_index::PathFilter,
    ) -> crate::languages::Skeleton {
        use crate::languages::chain::State;
        use crate::languages::{Skeleton, state_key};
        let file = gitterrace_index::StateFile::Chain;
        let key = state_key(
            &format!("{:?}", gitterrace_index::Options::default()),
            &self.ignore_files(head).unwrap_or_default(),
        );
        let prev = self.index.load_state(file, key).and_then(|b| State::decode(&b));
        let (sk, state) = prev
            .as_ref()
            .and_then(|p| Skeleton::extend(src, filter, p))
            .unwrap_or_else(|| Skeleton::build(src, filter));
        // A build over a read that failed is refused by the caller: its state is not kept.
        if src.failure().is_none()
            && prev.as_ref() != Some(&state)
            && let Err(e) = self.index.save_state(file, key, &state.encode())
        {
            tracing::debug!("{}: chain state not saved: {e}", self.name);
        }
        sk
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
        let found = self.ignore_files(commit)?;
        Some(gitterrace_git::IgnoreSet::from_files(found.iter().cloned()))
    }

    /// `(path, text)` of the ignore files committed at `commit`, found once per commit.
    pub(crate) fn ignore_files(&self, commit: &str) -> Option<Arc<Vec<(String, String)>>> {
        if let Ok(g) = self.caches.ignores.lock()
            && let Some((id, found)) = g.as_ref()
            && id == commit
        {
            return Some(Arc::clone(found));
        }
        let found = gitterrace_git::IgnoreSet::files_at(&self.repo, commit).ok()?;
        let found = Arc::new(found);
        if let Ok(mut g) = self.caches.ignores.lock() {
            *g = Some((commit.to_owned(), Arc::clone(&found)));
        }
        Some(found)
    }

    /// All-history totals of `commit` with `opts`, both populations, computed once per index
    /// generation over the tables every unfiltered page of `commit` shares: a fresh set of
    /// them cost the cold overview half a second on rust.
    pub(crate) fn totals(
        &self,
        commit: &str,
        src: &dyn gitterrace_index::feed::InsightsSource,
        opts: gitterrace_index::Options,
    ) -> crate::caches::OverviewTotals {
        let key = format!("{commit}\0{}\0{opts:?}", self.index.generation());
        if let Ok(g) = self.caches.totals.lock()
            && let Some((k, t)) = g.as_ref()
            && *k == key
        {
            return t.clone();
        }
        let ins = crate::insights::shared_on(self, src, commit, opts);
        let t = crate::caches::OverviewTotals {
            every: ins.every_totals(),
            counted: ins.period_summary(gitterrace_index::Period::All).current,
        };
        if src.failure().is_none()
            && let Ok(mut g) = self.caches.totals.lock()
        {
            *g = Some((key, t.clone()));
        }
        t
    }
}

/// The author time of the newest commit whose diff names each path.
fn last_touched(
    src: &dyn gitterrace_index::feed::InsightsSource,
) -> std::collections::HashMap<String, i64> {
    use gitterrace_index::feed::CommitRange;
    let times: std::collections::HashMap<_, _> =
        src.commits_in(CommitRange::All).map(|c| (c.rank, c.time)).collect();
    let ranks: Vec<_> = times.keys().copied().collect();
    let mut ranks = ranks;
    ranks.sort_unstable();
    let mut out = std::collections::HashMap::new();
    for (rank, rows) in src.rows_for(ranks) {
        let time = times.get(&rank).copied().unwrap_or_default();
        for f in &rows {
            if let Some(p) = src.path(f.path) {
                let t = out.entry(p.to_string()).or_insert(time);
                *t = (*t).max(time);
            }
        }
    }
    out
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
                    src.path(f.path).is_some_and(|p| crate::filter::under(&p, path))
                        || f.old
                            .and_then(|o| src.path(o))
                            .is_some_and(|o| crate::filter::under(&o, path))
                });
            let id = src.commit_id(rank).as_deref().unwrap_or_default().to_owned();
            let parents = src.parent_ids(rank).iter().map(ToString::to_string).collect();
            (id, parents, touched)
        })
        .collect();
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gitterrace_testkit::git;

    use crate::app::App;

    /// The finder lists recently changed files first. Files changed at the same time, or never
    /// touched since, keep their alphabetical order.
    #[test]
    fn file_paths_list_the_newest_touched_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        for (name, date) in [
            ("a.txt", "2020-01-01T00:00:00"),
            ("b.txt", "2020-01-02T00:00:00"),
            ("c.txt", "2020-01-03T00:00:00"),
        ] {
            std::fs::write(p.join(name), "x\n").expect("write");
            git(p, &["add", "."]);
            git(p, &["commit", "-q", "-m", "feat: x", "--date", date]);
        }
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        let head = app.repo.resolve("HEAD").expect("head");
        let before = app.file_paths(&head).expect("files");
        assert_eq!(*before, ["a.txt", "b.txt", "c.txt"], "no index yet");
        app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
        app.mark_ready();
        let after = app.file_paths(&head).expect("files");
        assert_eq!(*after, ["c.txt", "b.txt", "a.txt"]);
    }

    /// Guard class: a table derived from the source and kept by (head, generation) while a
    /// read failed. The rebuild of the same history keeps the generation, so a walk that
    /// finished after the rebuild cleared the caches (the warm-up's languages walk) was
    /// served as data over the clean cache. Every such table is refused here.
    #[test]
    fn nothing_built_over_a_failed_read_is_kept() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.rs"), "fn a() {}\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat: one"]);
        let repo = gitterrace_git::Repo::discover(p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = Arc::new(App::new(repo, index));
        app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready").unchecked();
        zero_blocks(app.index.dir());
        let filter = crate::languages::path_filter(&app, &head);
        app.chain(&head, &*src, &filter);
        app.totals(&head, &*src, gitterrace_index::Options::default());
        assert!(src.failure().is_some(), "the walks met the zeroed blocks");
        let kept_in = |c: &crate::caches::Caches| {
            [
                ("languages chain", c.chain.lock().is_ok_and(|g| g.is_some())),
                ("totals", c.totals.lock().is_ok_and(|g| g.is_some())),
            ]
        };
        for (what, kept) in kept_in(&app.caches) {
            assert!(!kept, "the {what} built over a failed read was kept");
        }
    }

    /// Zeroes every block byte of each segment and chain file in `cache`. Same length, so the
    /// set still opens; a read of any block fails its check.
    fn zero_blocks(cache: &std::path::Path) {
        for e in std::fs::read_dir(cache).expect("ls").flatten() {
            let seg = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let is_seg = seg.extension().is_some_and(|x| x == "gts");
            if !((name.starts_with("seg-") || name.starts_with("chain-")) && is_seg) {
                continue;
            }
            let mut bytes = std::fs::read(&seg).expect("read");
            let end = bytes.len() - 64;
            let toc = usize::try_from(u64::from_le_bytes(
                bytes[end + 16..end + 24].try_into().expect("8 bytes"),
            ))
            .expect("toc");
            bytes[16..toc].fill(0);
            std::fs::write(&seg, &bytes).expect("corrupt");
        }
    }
}
