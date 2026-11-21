//! Peeks at the shared state that only tests use.

use super::Env;

impl Env {
    pub(crate) fn free_jobs(&self) -> usize {
        self.jobs.available_permits()
    }
}
