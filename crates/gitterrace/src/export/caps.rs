//! The export's one page budget, diff fragments included.

use anyhow::bail;

use crate::export::Scope;

/// Pages written so far against the one budget, diff fragments included.
pub(crate) struct Caps {
    pub(crate) pages: usize,
    max: Option<usize>,
    commits: usize,
    file_kb: u64,
}

impl Caps {
    pub(crate) const fn new(max: Option<usize>, scope: &Scope) -> Self {
        Self {
            pages: 0,
            max,
            commits: scope.commits,
            file_kb: scope.max_file / 1024,
        }
    }

    pub(crate) fn take(&mut self, path: &str) -> anyhow::Result<()> {
        if let Some(max) = self.max.filter(|m| self.pages >= *m) {
            let kind = if is_fragment(path) {
                "a diff fragment"
            } else {
                "a page"
            };
            bail!(
                "export stopped at the cap of {} pages (diff fragments count) on {kind}, {path}; \
                 nothing was written. Lower --commits (now {}) or --file-kb (now {}), or \
                 raise --export-pages or limits.export_pages in config.toml",
                max,
                self.commits,
                self.file_kb
            );
        }
        self.pages += 1;
        Ok(())
    }
}

/// Whether `path` is a folded diff fragment, under any repository prefix.
pub(crate) fn is_fragment(path: &str) -> bool {
    let local = path
        .strip_prefix("/r/")
        .and_then(|r| r.split_once('/'))
        .map_or(path, |(_, rest)| rest);
    local.trim_start_matches('/').starts_with("diff/")
}
