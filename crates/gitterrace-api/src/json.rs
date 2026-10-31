//! Stable `snake_case` JSON for the git types, which carry no serde impls themselves.

use serde_json::{Value, json};
use gitterrace_git::{
    BlameLine, ChangeKind, CommitDetail, CommitSummary, FileDiff, LineKind, RefInfo, RefKind,
    Signature, TreeEntry, TreeEntryKind,
};

pub(crate) fn signature(s: &Signature) -> Value {
    json!({ "name": s.name, "email": s.email, "time": s.time, "offset": s.offset })
}

pub(crate) fn summary(c: &CommitSummary) -> Value {
    json!({
        "id": c.id,
        "parents": c.parents,
        "author": signature(&c.author),
        "committer": signature(&c.committer),
        "subject": c.subject,
    })
}

/// A commit with its file list. Hunks are included when `diff` is set, and cut off
/// after `max_lines` diff lines in total (`truncated` says so).
pub(crate) fn detail(d: &CommitDetail, diff: bool, max_lines: Option<usize>) -> Value {
    let mut budget = max_lines.unwrap_or(usize::MAX);
    let mut truncated = false;
    let files: Vec<Value> = d
        .files
        .iter()
        .map(|f| file_diff(f, diff, &mut budget, &mut truncated))
        .collect();
    let mut v = summary(&d.summary);
    v["body"] = json!(d.body);
    v["files"] = json!(files);
    v["truncated"] = json!(truncated);
    v
}

fn file_diff(f: &FileDiff, diff: bool, budget: &mut usize, truncated: &mut bool) -> Value {
    let kind = match f.kind {
        ChangeKind::Added => "added",
        ChangeKind::Modified => "modified",
        ChangeKind::Deleted => "deleted",
        ChangeKind::Renamed => "renamed",
    };
    let mut v = json!({
        "path": f.path,
        "kind": kind,
        "old_path": f.old_path,
        "additions": f.additions,
        "deletions": f.deletions,
        "binary": f.binary,
    });
    if diff {
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
            let lines: Vec<Value> = h.lines[..take]
                .iter()
                .map(|l| {
                    let k = match l.kind {
                        LineKind::Context => "context",
                        LineKind::Added => "added",
                        LineKind::Removed => "removed",
                    };
                    json!({ "kind": k, "old_no": l.old_no, "new_no": l.new_no, "text": l.text })
                })
                .collect();
            hunks.push(json!({ "header": h.header, "lines": lines }));
        }
        v["hunks"] = json!(hunks);
    }
    v
}

pub(crate) fn tree_entry(e: &TreeEntry) -> Value {
    let kind = match e.kind {
        TreeEntryKind::Dir => "dir",
        TreeEntryKind::File => "file",
        TreeEntryKind::Symlink => "symlink",
        TreeEntryKind::Submodule => "submodule",
    };
    json!({ "name": e.name, "kind": kind, "size": e.size })
}

pub(crate) fn ref_info(r: &RefInfo) -> Value {
    let kind = match r.kind {
        RefKind::Branch => "branch",
        RefKind::Tag => "tag",
        RefKind::Remote => "remote",
    };
    json!({ "name": r.name, "kind": kind, "target": r.target, "time": r.time })
}

pub(crate) fn blame_line(b: &BlameLine) -> Value {
    json!({
        "line_no": b.line_no,
        "commit": b.commit,
        "author": b.author,
        "time": b.time,
        "text": b.text,
    })
}
