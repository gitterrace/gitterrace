//! Reads that compare two revisions or the working tree against HEAD.

use gitterrace_types::api::{CompareResponse, WorktreeResponse};

use crate::bounds::{checked, checked_path};
use crate::core::{ApiState, under};
use crate::error::ApiError;
use crate::wire;

impl ApiState {
    /// The files that differ between `base` and `head`, hunks cut at the diff line cap.
    /// `path` keeps the files at or under it. `three_dot` compares from the merge base.
    pub fn compare(
        &self,
        base: &str,
        head: &str,
        path: Option<&str>,
        three_dot: bool,
        max_lines: Option<usize>,
    ) -> Result<CompareResponse, ApiError> {
        let (base, head) = (checked(base)?, checked(head)?);
        let path = path
            .map(|p| checked_path(p).map(|p| p.trim_matches('/')))
            .transpose()?
            .filter(|p| !p.is_empty());
        let (from, mut list) = self.repo.compare_files(base, head, three_dot)?;
        if let Some(p) = path {
            list.retain(|f| {
                under(&f.path, p) || f.old_path.as_deref().is_some_and(|o| under(o, p))
            });
        }
        let cap = self.diff_cap(max_lines);
        let (mut files, truncated) = wire::files(&list, true, cap);
        let head_id = self.repo.resolve(head)?;
        for f in &mut files {
            (f.permalink, f.link) = self.links(&head_id, Some(&f.path), None);
        }
        Ok(CompareResponse {
            additions: list.iter().map(|f| u64::from(f.additions)).sum(),
            base: base.to_owned(),
            deletions: list.iter().map(|f| u64::from(f.deletions)).sum(),
            files,
            from,
            head: head.to_owned(),
            path: path.map(str::to_owned),
            three_dot,
            truncated,
        })
    }

    /// Uncommitted changes against HEAD, hunks cut at the diff line cap. Reads this machine's
    /// working tree, so a committed-only (public) handle refuses it.
    pub fn worktree_diff(
        &self,
        scope: &str,
        max_lines: Option<usize>,
    ) -> Result<WorktreeResponse, ApiError> {
        if self.repo.is_committed_only() {
            return Err(ApiError::Forbidden("the working tree"));
        }
        let (scope, which) = match scope {
            "all" => ("all", gitterrace_git::Scope::All),
            "staged" => ("staged", gitterrace_git::Scope::Staged),
            "unstaged" => ("unstaged", gitterrace_git::Scope::Unstaged),
            other => {
                return Err(ApiError::BadRequest(format!(
                    "unknown scope {other:?}: use `staged`, `unstaged` or `all`"
                )));
            }
        };
        let list = gitterrace_git::worktree_diff_scoped(&self.repo, which)?;
        let (files, truncated) = wire::files(&list, true, self.diff_cap(max_lines));
        Ok(WorktreeResponse {
            additions: list.iter().map(|f| u64::from(f.additions)).sum(),
            deletions: list.iter().map(|f| u64::from(f.deletions)).sum(),
            files,
            scope: scope.to_owned(),
            truncated,
        })
    }
}
