//! A gitterrace permalink read back: the code it names at its commit and where that code is now.

use gitterrace_actions::Link;
use gitterrace_git::AnchorQuery;
use gitterrace_types::AnchorBasis;
use gitterrace_types::api::{FindingAnchor, FindingLines};
use serde::Serialize;

use crate::core::ApiState;
use crate::error::ApiError;

/// Most lines of context a caller may ask around a linked range. Not configurable: the blob
/// caps (`api.blob_lines`, `api.blob_kb`) bound the answer, this only bounds the request.
pub(crate) const MAX_CONTEXT: usize = 50;

/// What `resolve_permalink` answers. The text is repository content: untrusted.
#[derive(Debug, Serialize)]
pub(crate) struct ResolvedLink {
    /// Where the code is now, as for a finding. Null when the working tree switch is off,
    /// because the check reads the working tree.
    pub(crate) anchor: Option<FindingAnchor>,
    pub(crate) binary: bool,
    /// The full commit id the link names.
    pub(crate) commit: String,
    /// The lines `text` holds, null when it holds the file from its first line.
    pub(crate) lines: Option<FindingLines>,
    pub(crate) path: String,
    pub(crate) text: Option<String>,
    pub(crate) total_lines: Option<usize>,
    /// True when the answer is shorter than the link (plus context) asked for.
    pub(crate) truncated: bool,
}

fn index(n: u32) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

fn line_no(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

impl ApiState {
    /// The text `link` names, with `context` extra lines either side (at most 50), under the
    /// caps of `read_file`. The caller has matched the repository id.
    pub(crate) fn resolve_link(
        &self,
        link: &Link,
        context: Option<usize>,
    ) -> Result<ResolvedLink, ApiError> {
        let context = context.unwrap_or(0);
        if context > MAX_CONTEXT {
            return Err(ApiError::BadRequest(format!("context_lines is at most {MAX_CONTEXT}")));
        }
        let Some(path) = link.path.as_deref() else {
            return Err(ApiError::BadRequest(format!(
                "the link names commit {}, not a file: call show_commit",
                link.sha
            )));
        };
        let want = link.lines.map(|l| {
            let first = index(l.start).saturating_sub(context).max(1);
            (first, index(l.end).saturating_add(context))
        });
        let blob = self.blob(
            Some(&link.sha),
            path,
            want.map(|(first, _)| first),
            want.map(|(first, last)| last.saturating_sub(first) + 1),
        )?;
        if let (Some(l), Some(total)) = (link.lines, blob.total_lines)
            && index(l.start) > total
        {
            return Err(ApiError::BadRequest(format!(
                "the link starts at line {} but the file has {total} lines",
                l.start
            )));
        }
        let got = blob.text.as_deref().map_or(0, |t| t.lines().count());
        let truncated = match (want, blob.total_lines) {
            (Some((first, last)), Some(total)) => {
                let expected = last.min(total).saturating_sub(first) + 1;
                // `blob` cuts a text over the byte cap at a char boundary, at most 3 bytes
                // under it and keeps the line count, so a text that close counts as cut.
                let cap = self.settings().caps.blob_kb.saturating_mul(1024);
                got < expected || blob.text.as_ref().is_some_and(|t| t.len() + 3 >= cap)
            }
            _ => blob.truncated,
        };
        let lines = want.filter(|_| got > 0).map(|(first, _)| FindingLines {
            end: line_no(first + got - 1),
            start: line_no(first),
        });
        let anchor = if self.settings().worktree_on {
            self.link_anchor(&blob.commit, path, link)?
        } else {
            None
        };
        Ok(ResolvedLink {
            anchor,
            binary: blob.binary,
            commit: blob.commit,
            lines,
            path: blob.path,
            text: blob.text,
            total_lines: blob.total_lines,
            truncated,
        })
    }

    fn link_anchor(
        &self,
        sha: &str,
        path: &str,
        link: &Link,
    ) -> Result<Option<FindingAnchor>, ApiError> {
        let query = AnchorQuery {
            sha: sha.to_owned(),
            path: Some(path.to_owned()),
            lines: link.lines.map(|l| (l.start, l.end)),
        };
        let found = gitterrace_git::anchors_now(&self.repo, &[query])?;
        Ok(found.into_iter().next().map(|a| FindingAnchor {
            checked_against: if a.worktree { AnchorBasis::Worktree } else { AnchorBasis::Head },
            now_lines: a.now_lines.map(|(start, end)| FindingLines { end, start }),
            now_path: a.now_path,
            status: a.status,
        }))
    }
}
