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

const POLL: Duration = Duration::from_secs(3);
/// How long a dirty-worktree answer is reused, so every page need not run a status.
const DIRTY_TTL: Duration = Duration::from_secs(2);
/// Cap on cached directory listings and file lists; both are cheap to rebuild.
const CACHE_ENTRIES: usize = 256;

pub(crate) struct App {
    pub(crate) repo: Repo,
    pub(crate) index: Index,
    pub(crate) name: String,
    progress: Mutex<Progress>,
    ready: AtomicBool,
    records: Mutex<Option<(String, Arc<Vec<CommitRecord>>)>>,
    dirty: Mutex<Option<(Instant, bool)>>,
    file_lists: Mutex<HashMap<String, Arc<Vec<String>>>>,
    last_commits: Mutex<HashMap<String, Arc<Vec<LastCommit>>>>,
}

impl App {
    pub(crate) fn new(repo: Repo, index: Index) -> Self {
        let name = repo.name();
        Self {
            repo,
            index,
            name,
            progress: Mutex::new(Progress::default()),
            ready: AtomicBool::new(false),
            records: Mutex::new(None),
            dirty: Mutex::new(None),
            file_lists: Mutex::new(HashMap::new()),
            last_commits: Mutex::new(HashMap::new()),
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
        if let Ok(mut g) = self.records.lock() {
            *g = None;
        }
        self.ready.store(true, Ordering::Release);
    }

    /// Records reachable from `head`, parsed once per index generation. `None` until the first
    /// build finished.
    pub(crate) fn records(&self, head: &str) -> Result<Option<Arc<Vec<CommitRecord>>>> {
        if !self.ready() {
            return Ok(None);
        }
        if let Ok(g) = self.records.lock()
            && let Some((id, recs)) = g.as_ref()
            && id == head
        {
            return Ok(Some(Arc::clone(recs)));
        }
        let recs = Arc::new(self.index.records(&self.repo, head)?);
        if let Ok(mut g) = self.records.lock() {
            *g = Some((head.to_owned(), Arc::clone(&recs)));
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
    tokio::task::spawn_blocking(move || f(&app)).await?
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
                    last = head;
                }
                Ok(Err(e)) => tracing::warn!("index update failed: {e}"),
                Err(e) => tracing::warn!("index task failed: {e}"),
            }
        }
        tokio::time::sleep(POLL).await;
    }
}
