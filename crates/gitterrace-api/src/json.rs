//! Stable `snake_case` JSON for the git types, which carry no serde impls themselves.

use gitterrace_git::{
    BlameLine, CommitDetail, CommitSummary, FileDiff, RefInfo, Signature, TreeEntry,
};
use serde_json::{Value, json};

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
    let kind = f.kind.as_str();
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
                    json!({ "kind": l.kind.as_str(), "old_no": l.old_no, "new_no": l.new_no, "text": l.text })
                })
                .collect();
            hunks.push(json!({ "header": h.header, "lines": lines }));
        }
        v["hunks"] = json!(hunks);
    }
    v
}

pub(crate) fn tree_entry(e: &TreeEntry) -> Value {
    json!({ "name": e.name, "kind": e.kind.as_str(), "size": e.size })
}

pub(crate) fn ref_info(r: &RefInfo) -> Value {
    json!({ "name": r.name, "kind": r.kind.as_str(), "target": r.target, "time": r.time })
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
