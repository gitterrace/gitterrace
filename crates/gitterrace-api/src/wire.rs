//! Maps the git reads onto the response types of `gitterrace_types::api`. The git types carry
//! internal fields (modes, rewrite counts) the API never shows, so the mapping is explicit.

use gitterrace_git::{BlameLine, CommitSummary, FileDiff, RefInfo, TreeEntry};
use gitterrace_index::Contributor;
use gitterrace_types::api::{
    BlameItem, ContributorItem, DiffHunk, DiffLine, FileItem, LogCommit, RefItem, TreeItem,
};

/// A log row without links: the caller fills `permalink` and `link`.
pub(crate) fn log_commit(c: &CommitSummary) -> LogCommit {
    LogCommit {
        author: c.author.clone(),
        committer: c.committer.clone(),
        id: c.id.clone(),
        link: None,
        parents: c.parents.clone(),
        permalink: None,
        subject: c.subject.clone(),
    }
}

/// The files of `d`. Hunks are included when `diff` is set, and cut off after `max_lines` diff
/// lines in total. The flag says whether anything was cut.
pub(crate) fn files(list: &[FileDiff], diff: bool, max_lines: usize) -> (Vec<FileItem>, bool) {
    let mut budget = max_lines;
    let mut truncated = false;
    let items = list.iter().map(|f| file_item(f, diff, &mut budget, &mut truncated)).collect();
    (items, truncated)
}

fn file_item(f: &FileDiff, diff: bool, budget: &mut usize, truncated: &mut bool) -> FileItem {
    let hunks = diff.then(|| {
        let mut hunks = Vec::new();
        for h in &f.hunks {
            let take = h.lines.len().min(*budget);
            if take < h.lines.len() {
                *truncated = true;
            }
            *budget -= take;
            if take == 0 {
                break;
            }
            let lines = h.lines[..take]
                .iter()
                .map(|l| DiffLine {
                    kind: l.kind,
                    new_no: l.new_no,
                    old_no: l.old_no,
                    text: l.text.clone(),
                })
                .collect();
            hunks.push(DiffHunk { header: h.header.clone(), lines });
        }
        hunks
    });
    FileItem {
        additions: f.additions,
        binary: f.binary,
        deletions: f.deletions,
        hunks,
        kind: f.kind,
        link: None,
        old_path: f.old_path.clone(),
        path: f.path.clone(),
        permalink: None,
    }
}

pub(crate) fn tree_item(e: &TreeEntry) -> TreeItem {
    TreeItem { kind: e.kind, name: e.name.clone(), size: e.size }
}

pub(crate) fn ref_item(r: &RefInfo) -> RefItem {
    RefItem { kind: r.kind, name: r.name.clone(), target: r.target.clone(), time: r.time }
}

pub(crate) fn blame_item(b: &BlameLine) -> BlameItem {
    BlameItem {
        author: b.author.clone(),
        commit: b.commit.clone(),
        line_no: b.line_no,
        text: b.text.clone(),
        time: b.time,
    }
}

pub(crate) fn contributor(c: Contributor) -> ContributorItem {
    ContributorItem {
        active_days: c.active_days,
        additions: c.additions,
        bot: c.bot,
        commits: c.commits,
        deletions: c.deletions,
        emails: c.emails,
        first_commit: c.first_commit,
        first_time: c.first_time,
        last_commit: c.last_commit,
        name: c.name,
        series: c.series,
    }
}
