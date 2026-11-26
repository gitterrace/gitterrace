//! Reads that compare two revisions or the working tree against HEAD.

use gitterrace_git::FileDiff;
use gitterrace_types::api::{CompareResponse, WorktreeResponse};

use crate::bounds::{checked, checked_path};
use crate::core::{ApiState, under};
use crate::error::ApiError;
use crate::wire;

/// A checkpoint id and what differs from it.
pub(super) type SinceRead = (String, gitterrace_git::SinceDiff);

/// The files of the working tree, whether the list may be cut, and the checkpoint compared.
pub(super) struct WorktreeList {
    pub files: Vec<FileDiff>,
    pub files_omitted: bool,
    pub since: Option<SinceRead>,
}

/// Added and removed lines over a file list.
fn totals(list: &[FileDiff]) -> (u64, u64) {
    list.iter().fold((0, 0), |(a, d), f| (a + u64::from(f.additions), d + u64::from(f.deletions)))
}

impl ApiState {
    /// The files that differ between `base` and `head`, hunks cut at the diff line cap.
    /// `path` keeps the files at or under it. `three_dot` compares from the merge base.
    pub fn compare(
        &self,
        base: &str,
        head: Option<&str>,
        path: Option<&str>,
        three_dot: Option<bool>,
        max_lines: Option<usize>,
    ) -> Result<CompareResponse, ApiError> {
        let (base, head) = (checked(base)?, checked(head.unwrap_or("HEAD"))?);
        let three_dot = three_dot.unwrap_or(false);
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
        let (additions, deletions) = totals(&list);
        Ok(CompareResponse {
            additions,
            base: base.to_owned(),
            deletions,
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
        scope: Option<&str>,
        max_lines: Option<usize>,
    ) -> Result<WorktreeResponse, ApiError> {
        self.worktree_diff_since(scope, max_lines, None)
    }

    /// [`ApiState::worktree_diff`] against a checkpoint when `since` names one: only the files
    /// whose content differs from it, `unchanged_files` counting the rest and `content_gone` the
    /// files whose copy was dropped (those diff against HEAD). A checkpoint holds the whole
    /// working tree, so `since` needs scope `all`.
    pub fn worktree_diff_since(
        &self,
        scope: Option<&str>,
        max_lines: Option<usize>,
        since: Option<&str>,
    ) -> Result<WorktreeResponse, ApiError> {
        let WorktreeList { files: list, files_omitted, since: since_part } =
            self.worktree_list(scope, since)?;
        let (files, truncated) = wire::files(&list, true, self.diff_cap(max_lines));
        let (additions, deletions) = totals(&list);
        let baseline_missing = since_part.as_ref().is_some_and(|(_, d)| d.baseline_missing);
        let history_unreadable = since_part.as_ref().is_some_and(|(_, d)| d.history_unreadable);
        let (since, unchanged_files, content_gone) = since_part
            .map_or((None, None, Vec::new()), |(id, d)| {
                (Some(id), Some(d.unchanged as u64), d.gone)
            });
        Ok(WorktreeResponse {
            additions,
            baseline_missing,
            content_gone,
            deletions,
            files,
            files_omitted,
            history_unreadable,
            scope: scope.unwrap_or("all").to_owned(),
            since,
            truncated,
            unchanged_files,
        })
    }

    /// The changed files of the working tree in `scope`, or the ones that differ from checkpoint
    /// `since`. Refused on a committed-only (public) handle.
    pub(super) fn worktree_list(
        &self,
        scope: Option<&str>,
        since: Option<&str>,
    ) -> Result<WorktreeList, ApiError> {
        if self.repo.is_committed_only() {
            return Err(ApiError::Forbidden("the working tree"));
        }
        let which = match scope.unwrap_or("all") {
            "all" => gitterrace_git::Scope::All,
            "staged" => gitterrace_git::Scope::Staged,
            "unstaged" => gitterrace_git::Scope::Unstaged,
            other => {
                return Err(ApiError::BadRequest(format!(
                    "unknown scope {other:?}: use `staged`, `unstaged` or `all`"
                )));
            }
        };
        let (list, files_omitted, since_part) = match since {
            None => {
                let l = gitterrace_git::worktree_listed(&self.repo, which)?;
                (l.files, l.files_omitted, None)
            }
            Some(id) => {
                if which != gitterrace_git::Scope::All {
                    return Err(ApiError::BadRequest(
                        "since compares the whole working tree: use scope `all`".to_owned(),
                    ));
                }
                let cp = self
                    .index
                    .checkpoint(id)?
                    .ok_or_else(|| ApiError::CheckpointNotFound(id.to_owned()))?;
                let d = gitterrace_git::worktree_diff_since(
                    &self.repo,
                    &cp.files,
                    &cp.head,
                    cp.files_omitted,
                )?;
                (d.files.clone(), d.files_omitted, Some((id.to_owned(), d)))
            }
        };
        Ok(WorktreeList { files: list, files_omitted, since: since_part })
    }
}
