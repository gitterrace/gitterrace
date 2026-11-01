//! Shared state and the background index refresher.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use gitterrace_git::{LastCommit, Repo};
use gitterrace_index::{CommitRecord, Index, Progress};

use crate::error::Result;

const POLL: Duration = Duration::from_secs(1);
/// Rendered stats pages are reused this long; time-relative windows move slowly.
const MEMO_TTL: Duration = Duration::from_secs(60);
const MEMO_ENTRIES: usize = 64;
/// How long a dirty-worktree answer is reused, so every page need not run a status.
const DIRTY_TTL: Duration = Duration::from_secs(2);
/// Cap on cached directory listings and file lists; both are cheap to rebuild.
const CACHE_ENTRIES: usize = 256;

/// Commit subjects by full id.
pub(crate) type Subjects = HashMap<String, String>;

/// Parsed records keyed by (head, index generation).
type RecordsEntry = ((String, u64), Arc<Vec<CommitRecord>>);

pub(crate) struct App {
    pub(crate) repo: Repo,
    pub(crate) index: Index,
    pub(crate) name: String,
    /// URL prefix: empty for a single repository, `/r/NAME` when several share the server.
    pub(crate) base: String,
    /// Name run requests use for this repo: the URL name with several repos.
    pub(crate) key: String,
    /// Set when actions are on: pages embed the token and the menu.
    pub(crate) actions: Option<Arc<crate::actions::Live>>,
    progress: Mutex<Progress>,
    ready: AtomicBool,
    /// Selected commits outside HEAD's history, queued for the one indexer loop so the cache
    /// file never has two writers.
    wanted: Mutex<std::collections::BTreeSet<String>>,
    records: Mutex<Option<RecordsEntry>>,
    memo: Mutex<HashMap<String, (Instant, Arc<String>)>>,
    dirty: Mutex<Option<(Instant, bool)>>,
    file_lists: Mutex<HashMap<String, Arc<Vec<String>>>>,
    last_commits: Mutex<HashMap<String, Arc<Vec<LastCommit>>>>,
    chain: Mutex<Option<(String, Arc<crate::languages::Chain>)>>,
    subjects: Mutex<Option<(String, Arc<Subjects>)>>,
    head_lines: Mutex<Option<(String, Arc<Vec<gitterrace_index::HeadFile>>)>>,
}

impl App {
    pub(crate) fn new(repo: Repo, index: Index) -> Self {
        let name = repo.name();
        Self {
            repo,
            index,
            key: name.clone(),
            name,
            base: String::new(),
            actions: None,
            progress: Mutex::new(Progress::default()),
            ready: AtomicBool::new(false),
            wanted: Mutex::new(std::collections::BTreeSet::new()),
            records: Mutex::new(None),
            memo: Mutex::new(HashMap::new()),
            dirty: Mutex::new(None),
            file_lists: Mutex::new(HashMap::new()),
            last_commits: Mutex::new(HashMap::new()),
            chain: Mutex::new(None),
            subjects: Mutex::new(None),
            head_lines: Mutex::new(None),
        }
    }

    /// Whether the worktree differs from HEAD, reused for a couple of seconds.
    pub(crate) fn dirty(&self) -> bool {
        if let Ok(g) = self.dirty.lock()
            && let Some((at, v)) = *g
            && at.elapsed() < DIRTY_TTL
        {
            return v;
        }
        let v = gitterrace_git::is_dirty(&self.repo);
        if let Ok(mut g) = self.dirty.lock() {
            *g = Some((Instant::now(), v));
        }
        v
    }

    /// Paths of all files at a full commit id. Commits are immutable, so entries never expire.
    pub(crate) fn file_paths(&self, commit: &str) -> Result<Arc<Vec<String>>> {
        if let Ok(g) = self.file_lists.lock()
            && let Some(v) = g.get(commit)
        {
            return Ok(Arc::clone(v));
        }
        let list: Vec<String> = gitterrace_git::files(&self.repo, commit)?
            .into_iter()
            .map(|f| f.path)
            .collect();
        let list = Arc::new(list);
        if let Ok(mut g) = self.file_lists.lock() {
            if g.len() >= CACHE_ENTRIES {
                g.clear();
            }
            g.insert(commit.to_owned(), Arc::clone(&list));
        }
        Ok(list)
    }

    /// Last commit per entry of `dir`, cached by the directory's tree id.
    pub(crate) fn last_commits(&self, commit: &str, dir: &str) -> Result<Arc<Vec<LastCommit>>> {
        let key = gitterrace_git::tree_id(&self.repo, commit, dir)?;
        if let Ok(g) = self.last_commits.lock()
            && let Some(v) = g.get(&key)
        {
            return Ok(Arc::clone(v));
        }
        let v = Arc::new(gitterrace_git::last_commits(
            &self.repo,
            commit,
            dir,
            Duration::from_secs(20),
        )?);
        if let Ok(mut g) = self.last_commits.lock() {
            if g.len() >= CACHE_ENTRIES {
                g.clear();
            }
            g.insert(key, Arc::clone(&v));
        }
        Ok(v)
    }

    /// Lines of code per language at every first-parent commit of `head`, built once per head.
    pub(crate) fn chain(
        &self,
        head: &str,
        records: &[CommitRecord],
    ) -> Arc<crate::languages::Chain> {
        if let Ok(g) = self.chain.lock()
            && let Some((id, c)) = g.as_ref()
            && id == head
        {
            return Arc::clone(c);
        }
        let ignore = gitterrace_git::IgnoreSet::at(&self.repo, head).ok();
        let ins =
            gitterrace_index::Insights::new(records, gitterrace_index::Options::default(), ignore);
        let subjects = self.subjects(head);
        let c = Arc::new(crate::languages::Chain::build(records, &ins, &subjects));
        if let Ok(mut g) = self.chain.lock() {
            *g = Some((head.to_owned(), Arc::clone(&c)));
        }
        c
    }

    /// Line count of every file in `head`, built once per head. Reading the line cache,
    /// walking the tree and saving the cache cost tens of ms per insights page otherwise.
    pub(crate) fn head_lines(&self, head: &str) -> Arc<Vec<gitterrace_index::HeadFile>> {
        if let Ok(g) = self.head_lines.lock()
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
        if let Ok(mut g) = self.head_lines.lock() {
            *g = Some((head.to_owned(), Arc::clone(&files)));
        }
        files
    }

    /// Commit subjects by id for everything reachable from `head`, scanned once per head.
    pub(crate) fn subjects(&self, head: &str) -> Arc<HashMap<String, String>> {
        if let Ok(g) = self.subjects.lock()
            && let Some((id, m)) = g.as_ref()
            && id == head
        {
            return Arc::clone(m);
        }
        let mut map: HashMap<String, String> = HashMap::new();
        if let Err(e) = gitterrace_git::scan_commits(&self.repo, head, |s, _| {
            map.insert(s.id.clone(), s.subject.clone());
            true
        }) {
            tracing::warn!("subjects unavailable: {e}");
        }
        let m = Arc::new(map);
        if let Ok(mut g) = self.subjects.lock() {
            *g = Some((head.to_owned(), Arc::clone(&m)));
        }
        m
    }

    /// A rendered page for `key`, if fresh.
    pub(crate) fn memo_get(&self, key: &str) -> Option<Arc<String>> {
        let g = self.memo.lock().ok()?;
        let (at, html) = g.get(key)?;
        (at.elapsed() < MEMO_TTL).then(|| Arc::clone(html))
    }

    pub(crate) fn memo_put(&self, key: String, html: Arc<String>) {
        if let Ok(mut g) = self.memo.lock() {
            if g.len() >= MEMO_ENTRIES {
                g.clear();
            }
            g.insert(key, (Instant::now(), html));
        }
    }

    /// The query reduced to what the stats pages read, sorted: unknown parameters and the
    /// order of known ones must not make a new memo entry.
    pub(crate) fn canonical_query(raw: Option<&str>) -> String {
        const KNOWN: [&str; 12] = [
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

    fn set_progress(&self, p: Progress) {
        if let Ok(mut g) = self.progress.lock() {
            *g = p;
        }
    }

    pub(crate) fn progress(&self) -> Progress {
        self.progress.lock().map(|g| *g).unwrap_or_default()
    }

    /// True once one full pass over HEAD finished, so records can be trusted.
    pub(crate) fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    pub(crate) fn mark_ready(&self) {
        self.ready.store(true, Ordering::Release);
    }

    /// Records reachable from `head`, parsed once per index generation. `None` until the first
    /// build finished, and for a commit outside HEAD's history until its missing commits are
    /// indexed; the indexer loop picks such a commit up from `wanted`.
    pub(crate) fn records(&self, head: &str) -> Result<Option<Arc<Vec<CommitRecord>>>> {
        if !self.ready() {
            return Ok(None);
        }
        let key = (head.to_owned(), self.index.generation());
        if let Ok(g) = self.records.lock()
            && let Some((k, recs)) = g.as_ref()
            && *k == key
        {
            return Ok(Some(Arc::clone(recs)));
        }
        if self.index.pending(&self.repo, head)? > 0 {
            if let Ok(mut w) = self.wanted.lock() {
                w.insert(head.to_owned());
            }
            return Ok(None);
        }
        let recs = Arc::new(self.index.records(&self.repo, head)?);
        if let Ok(mut g) = self.records.lock() {
            *g = Some((key, Arc::clone(&recs)));
        }
        Ok(Some(recs))
    }
}

/// Runs git or index work off the async threads.
pub(crate) async fn blocking<T: Send + 'static>(
    app: &Arc<App>,
    f: impl FnOnce(&App) -> Result<T> + Send + 'static,
) -> Result<T> {
    let app = Arc::clone(app);
    let rev = crate::fmt::REQUEST_REV
        .try_with(Clone::clone)
        .unwrap_or_else(|_| "HEAD".to_owned());
    tokio::task::spawn_blocking(move || {
        crate::fmt::with_base(&app.base, || crate::fmt::with_rev(&rev, || f(&app)))
    })
    .await?
}

/// Re-indexes whenever HEAD moves. Browsing never waits on this task.
pub(crate) async fn keep_index_fresh(app: Arc<App>) {
    let mut last: Option<String> = None;
    loop {
        let head = blocking(&app, |a| Ok(a.repo.resolve("HEAD")?)).await.ok();
        if head.is_some() && head != last {
            let a = Arc::clone(&app);
            let run = tokio::task::spawn_blocking(move || {
                a.index.update(&a.repo, "HEAD", &|p| a.set_progress(p))
            })
            .await;
            match run {
                Ok(Ok(())) => {
                    app.mark_ready();
                    if let Some(h) = head.clone() {
                        let a = Arc::clone(&app);
                        // Warm what the first visitor would otherwise pay for.
                        tokio::task::spawn_blocking(move || {
                            if let Ok(Some(r)) = a.records(&h) {
                                a.chain(&h, &r);
                            }
                        });
                    }
                    last = head;
                }
                Ok(Err(e)) => tracing::warn!("index update failed: {e}"),
                Err(e) => tracing::warn!("index task failed: {e}"),
            }
        }
        tokio::time::sleep(POLL).await;
        let wanted = app
            .wanted
            .lock()
            .map(|mut w| std::mem::take(&mut *w))
            .unwrap_or_default();
        for rev in wanted {
            let a = Arc::clone(&app);
            let run = tokio::task::spawn_blocking(move || {
                a.index.update(&a.repo, &rev, &|p| a.set_progress(p))
            })
            .await;
            if let Ok(Err(e)) | Err(e) = run.map_err(std::io::Error::other) {
                tracing::warn!("index update for a selected ref failed: {e}");
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::process::Command;

    use super::App;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@e.x"])
            .args(args)
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn junk_and_order_do_not_make_new_memo_keys() {
        let canon = |q| App::canonical_query(Some(q));
        assert_eq!(canon("x=1&type=fix"), canon("type=fix&x=2&y"));
        assert_eq!(canon("period=1y&type=fix"), canon("type=fix&period=1y"));
        assert_ne!(canon("type=fix"), canon("type=feat"));
        assert_eq!(App::canonical_query(None), "");
    }

    #[test]
    fn memo_key_moves_with_head_and_index_and_records_follow() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main"]);
        std::fs::write(path.join("a.txt"), "1\n").expect("write");
        git(path, &["add", "."]);
        git(path, &["commit", "-q", "-m", "feat: one"]);
        let repo = gitterrace_git::Repo::discover(path).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let key = app.memo_key("t", Some("type=fix"));
        assert_ne!(key, app.memo_key("t", Some("type=feat")));
        app.memo_put(key.clone(), std::sync::Arc::new("page".into()));
        assert!(app.memo_get(&key).is_some());
        let first = app.records(&head).expect("records").expect("ready");
        assert_eq!(first.len(), 1);

        std::fs::write(path.join("a.txt"), "2\n").expect("write");
        git(path, &["commit", "-qam", "fix: two"]);
        let moved = app.memo_key("t", Some("type=fix"));
        assert_ne!(key, moved, "moving HEAD must change the memo key");
        assert!(app.memo_get(&moved).is_none());
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let second = app.records(&head).expect("records").expect("ready");
        assert_eq!(second.len(), 2, "records cache must follow the new tip");
    }
}
