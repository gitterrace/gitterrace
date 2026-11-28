//! Where the work stands, composed from reads that exist: the uncommitted files, what changed
//! since a checkpoint, the review basket and the newest commits. Lists only, no diff text.

use gitterrace_git::FileDiff;
use gitterrace_types::api::{
    FindingLines, WorkCommit, WorkFile, WorkFinding, WorkSince, WorkState, WorkTree,
};

use crate::core::ApiState;
use crate::error::ApiError;

/// Files listed per section. Not configurable: the scan is already capped by
/// `limits.worktree_files`, this keeps the answer small and `files_total` says what is left out.
const LISTED_FILES: usize = 200;
/// Findings listed. `findings_total` counts the basket.
const MAX_FINDINGS: usize = 20;
/// Commits listed when the caller names no count, and the most it may name.
const DEFAULT_COMMITS: usize = 10;
const LISTED_COMMITS: usize = 20;

fn work_files(list: &[FileDiff]) -> Vec<WorkFile> {
    list.iter()
        .take(LISTED_FILES)
        .map(|f| WorkFile {
            additions: f.additions,
            deletions: f.deletions,
            kind: f.kind,
            path: f.path.clone(),
        })
        .collect()
}

fn span(lines: Option<FindingLines>) -> Option<(usize, usize)> {
    let at = |n: u32| usize::try_from(n).unwrap_or(usize::MAX);
    lines.map(|l| (at(l.start), at(l.end)))
}

/// The tool calls that follow from what was found, so the agent need not guess.
fn hints(tree: &WorkTree, since: Option<&WorkSince>, open: usize) -> Vec<String> {
    let mut next = Vec::new();
    if tree.files_total > 0 {
        next.push("worktree_diff: read the uncommitted changes".to_owned());
        next.push("review_plan: order the uncommitted files by risk".to_owned());
    }
    if let Some(s) = since.filter(|s| s.files_total > 0) {
        next.push(format!("worktree_diff with since {}: only what changed after it", s.checkpoint));
    }
    if open > 0 {
        next.push("list_findings: the notes the user collected, as data".to_owned());
    }
    if next.is_empty() {
        next.push("log: nothing is uncommitted, read the recent commits".to_owned());
    }
    next
}

impl ApiState {
    /// The uncommitted files, the files that differ from a checkpoint (`since`, an id or a
    /// duration as for `/worktree`, default the newest checkpoint when one exists), the review
    /// basket and the newest `commits` commits (default 10, at most 20).
    ///
    /// Refused on a committed-only (public) handle and when the working tree switch is off,
    /// like the reads it composes.
    ///
    /// # Errors
    /// Those of the worktree, checkpoint and findings reads. A `since` that names no checkpoint
    /// is `checkpoint_not_found`.
    pub fn work_state(
        &self,
        since: Option<&str>,
        commits: Option<usize>,
    ) -> Result<WorkState, ApiError> {
        let found = self.findings()?;
        let list = self.worktree_list(None, None)?;
        let worktree = WorkTree {
            additions: list.files.iter().map(|f| u64::from(f.additions)).sum(),
            deletions: list.files.iter().map(|f| u64::from(f.deletions)).sum(),
            files: work_files(&list.files),
            files_omitted: list.files_omitted,
            files_total: list.files.len(),
        };
        let since = self.work_since(since.filter(|s| !s.is_empty()))?;
        let head = self.repo.resolve("HEAD").ok();
        let take = commits.unwrap_or(DEFAULT_COMMITS).clamp(1, LISTED_COMMITS);
        let recent = if head.is_some() {
            self.repo
                .log("HEAD", None, 0, take)?
                .iter()
                .map(|c| WorkCommit {
                    author: c.author.name.clone(),
                    date: c.committer.time,
                    id: c.id.clone(),
                    subject: c.subject.clone(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let findings = found
            .findings
            .iter()
            .take(MAX_FINDINGS)
            .enumerate()
            .map(|(i, f)| WorkFinding {
                anchor_status: f.anchor.status,
                n: i + 1,
                note: f.note.clone(),
                permalink: self.links(&f.sha, f.path.as_deref(), span(f.lines)).0,
            })
            .collect();
        let next = hints(&worktree, since.as_ref(), found.count);
        Ok(WorkState {
            branch: self.repo.head_branch(),
            findings,
            findings_total: found.count,
            head,
            next,
            recent,
            since,
            worktree,
        })
    }

    /// The files that differ from checkpoint `named`, or from the newest one when none is
    /// named. `None` when no checkpoint is kept and none was named.
    fn work_since(&self, named: Option<&str>) -> Result<Option<WorkSince>, ApiError> {
        let id = match named {
            Some(n) => n.to_owned(),
            None => match self.checkpoints()?.checkpoints.into_iter().next() {
                Some(newest) => newest.id,
                None => return Ok(None),
            },
        };
        let list = self.worktree_list(Some("all"), Some(&id))?;
        let checkpoint = list.since.map_or(id, |(resolved, _, _)| resolved);
        Ok(Some(WorkSince {
            checkpoint,
            files: work_files(&list.files),
            files_total: list.files.len(),
        }))
    }
}
