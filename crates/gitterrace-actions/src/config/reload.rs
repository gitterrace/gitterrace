//! The live action set and its reload on file change.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use crate::config::Config;

/// Holds the live set and reloads it when the file changes. Mounting the route is a startup
/// decision made by the caller; a reload only swaps the set, never mints a token.
pub struct Reloader {
    path: PathBuf,
    repos: Vec<PathBuf>,
    stamp: Option<(SystemTime, u64)>,
    current: Arc<RwLock<Arc<Config>>>,
    error: Option<String>,
}

impl Reloader {
    pub fn new(path: PathBuf, repos: Vec<PathBuf>, initial: Config) -> Self {
        let stamp = stamp(&path);
        Self { path, repos, stamp, current: Arc::new(RwLock::new(Arc::new(initial))), error: None }
    }

    /// Why the last reload was refused; `None` once a later one succeeded.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The shared handle the run route reads from.
    pub fn handle(&self) -> Arc<RwLock<Arc<Config>>> {
        Arc::clone(&self.current)
    }

    /// One poll: when the file changed, a valid file replaces the set and an invalid one
    /// keeps it. Returns the log lines.
    pub fn poll(&mut self) -> Vec<String> {
        let now = stamp(&self.path);
        if now == self.stamp {
            return Vec::new();
        }
        self.stamp = now;
        match Config::load(&self.path, &self.repos) {
            Ok(new) => {
                self.error = None;
                let Ok(mut cur) = self.current.write() else {
                    return vec!["actions: config lock poisoned, reload skipped".into()];
                };
                let lines = cur.diff(&new);
                *cur = Arc::new(new);
                lines
            }
            Err(e) => {
                self.error = Some(e.to_string());
                vec![format!("actions: reload refused, keeping the old set: {e}")]
            }
        }
    }
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let m = std::fs::symlink_metadata(path).ok()?;
    Some((m.modified().ok()?, m.len()))
}
