//! Shared state and the background index refresher.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gitterrace_git::Repo;
use gitterrace_index::{CommitRecord, Index, Progress};

use crate::error::Result;

const POLL: Duration = Duration::from_secs(3);

pub(crate) struct App {
    pub(crate) repo: Repo,
    pub(crate) index: Index,
    pub(crate) name: String,
    progress: Mutex<Progress>,
    ready: AtomicBool,
    records: Mutex<Option<(String, Arc<Vec<CommitRecord>>)>>,
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
        }
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
