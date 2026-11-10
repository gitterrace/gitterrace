//! Peeks at the shared state that only tests use.

use std::sync::Arc;

use super::{App, Env};

impl Env {
    pub(crate) fn free_jobs(&self) -> usize {
        self.jobs.available_permits()
    }

    /// Every live app built on this environment.
    pub(crate) fn apps_for_test(&self) -> Vec<Arc<App>> {
        self.apps
            .lock()
            .map(|g| g.iter().filter_map(std::sync::Weak::upgrade).collect())
            .unwrap_or_default()
    }
}
