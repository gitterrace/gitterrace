//! What the configuration and the server put in force for every clone of a state: link
//! style, counting defaults, answer caps and the blocking-read slots.

use std::sync::Arc;

use gitterrace_index::Options;
use gitterrace_types::LinkStyle;

use crate::core::{ApiState, options_from};

#[derive(Clone)]
pub(super) struct Settings {
    /// Form of the `link` field; `permalink` is always `/go`.
    pub(super) style: LinkStyle,
    /// Counting options when a request names none: the `insights` configuration, the same
    /// defaults the web pages start from.
    pub(super) defaults: Options,
    /// How much one answer may carry, from `[api]` (see `gitterrace_config::Api`): a vendored
    /// commit holds millions of diff lines, one minified line can be megabytes, and a path
    /// filter walks every skipped commit.
    pub(super) caps: gitterrace_config::Api,
    /// Checkpoint caps from `[review]`.
    pub(super) review: gitterrace_config::Review,
    /// `limits.diff_fold_lines`: where the size term of a review plan saturates.
    pub(super) fold_lines: usize,
    /// `worktree_enabled` for this repository: the working-tree source of a review plan.
    pub(super) worktree_on: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let cfg = gitterrace_config::Config::default();
        Self {
            style: LinkStyle::default(),
            defaults: Options::default(),
            caps: cfg.api,
            review: cfg.review,
            fold_lines: cfg.limits.diff_fold_lines,
            worktree_on: true,
        }
    }
}

impl ApiState {
    /// Runs every blocking read of this state and its clones under `jobs`.
    pub fn share_jobs(&self, jobs: Arc<tokio::sync::Semaphore>) {
        match self.jobs.write() {
            Ok(mut g) => *g = jobs,
            Err(e) => *e.into_inner() = jobs,
        }
    }

    /// One blocking-read slot. The caller moves it into the blocking task, so a request that
    /// is dropped or times out frees it only once the read itself has exited.
    pub(crate) async fn job_slot(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let jobs = match self.jobs.read() {
            Ok(g) => Arc::clone(&g),
            Err(e) => Arc::clone(&e.into_inner()),
        };
        jobs.acquire_owned().await.ok()
    }

    /// Link style and counting defaults from the configuration: `links.style`, `insights.*`
    /// and this repository's `[repo.NAME] exclude`. The web's `/api/v1` and the MCP server both
    /// build their state through here, so the two answer with the same numbers.
    #[must_use]
    pub fn configured(self, cfg: &gitterrace_config::Config) -> Self {
        self.reconfigure(cfg);
        self
    }

    /// Puts `cfg` in force for this state and every clone of it, such as a router built
    /// earlier: a reloaded `config.toml` changes the next answer.
    pub fn reconfigure(&self, cfg: &gitterrace_config::Config) {
        self.index.set_block_cache(cfg.cache_bytes(gitterrace_config::Cache::Blocks));
        self.index.set_message_index(cfg.index.has_messages);
        let name = self.repo.name();
        let defaults = options_from(cfg, &name);
        let mut s = self.write_settings();
        s.style = cfg.links.style;
        s.defaults = defaults;
        s.caps = cfg.api.clone();
        s.review = cfg.review.clone();
        s.fold_lines = cfg.limits.diff_fold_lines;
        s.worktree_on = cfg.feature_on(gitterrace_config::Feature::Worktree, Some(&name));
    }

    pub(super) fn write_settings(&self) -> std::sync::RwLockWriteGuard<'_, Settings> {
        self.settings.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}
