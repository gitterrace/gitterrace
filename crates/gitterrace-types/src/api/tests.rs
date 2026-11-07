//! Each response serializes to the exact bytes of the `json!` literal it replaced, so the
//! field order (alphabetical, like a sorted map) is pinned.

use serde_json::json;

use crate::api::{
    BlameItem, BlameResponse, BlobResponse, CommitResponse, ContributorItem, ContributorsResponse,
    DiffHunk, DiffLine, EndpointDoc, FileItem, IndexResponse, InsightList, InsightRow, LogCommit,
    LogResponse, ParamDoc, RefItem, RefsResponse, RepoResponse, RepoTotals, Signature, TreeItem,
    TreeResponse,
};
use crate::{ChangeKind, LineKind, RefKind, TreeEntryKind};

fn person(name: &str) -> Signature {
    Signature {
        email: format!("{name}@x.io"),
        name: name.into(),
        offset: 3600,
        time: 1_700_000_000,
    }
}

fn person_json(name: &str) -> serde_json::Value {
    json!({ "name": name, "email": format!("{name}@x.io"), "time": 1_700_000_000, "offset": 3600 })
}

fn same<T: serde::Serialize>(typed: &T, old: &serde_json::Value) {
    assert_eq!(
        serde_json::to_string(typed).expect("typed"),
        serde_json::to_string(old).expect("old")
    );
}

#[test]
fn log_matches_the_map_it_replaced() {
    let typed = LogResponse {
        commits: vec![LogCommit {
            author: person("ann"),
            committer: person("bob"),
            id: "abc".into(),
            link: Some("gitterrace://x".into()),
            parents: vec!["p1".into()],
            permalink: Some("/go/x".into()),
            subject: "s".into(),
        }],
        has_more: true,
        page: 2,
        path: None,
        per_page: 30,
        rev: "HEAD".into(),
    };
    let old = json!({
        "rev": "HEAD", "path": null, "page": 2, "per_page": 30, "has_more": true,
        "commits": [{
            "id": "abc", "parents": ["p1"], "author": person_json("ann"),
            "committer": person_json("bob"), "subject": "s",
            "permalink": "/go/x", "link": "gitterrace://x",
        }],
    });
    same(&typed, &old);
}

#[test]
fn commit_matches_the_map_it_replaced() {
    let file = |hunks| FileItem {
        additions: 2,
        binary: false,
        deletions: 1,
        hunks,
        kind: ChangeKind::Renamed,
        link: None,
        old_path: Some("a".into()),
        path: "b".into(),
        permalink: None,
    };
    let typed = CommitResponse {
        author: person("ann"),
        body: "body".into(),
        committer: person("bob"),
        files: vec![
            file(Some(vec![DiffHunk {
                header: "@@".into(),
                lines: vec![DiffLine {
                    kind: LineKind::Added,
                    new_no: Some(1),
                    old_no: None,
                    text: "t".into(),
                }],
            }])),
            file(None),
        ],
        id: "abc".into(),
        link: None,
        parents: vec![],
        permalink: None,
        subject: "s".into(),
        truncated: false,
    };
    let old_file = json!({
        "path": "b", "kind": "renamed", "old_path": "a", "additions": 2, "deletions": 1,
        "binary": false, "permalink": null, "link": null,
    });
    let mut with_hunks = old_file.clone();
    with_hunks["hunks"] = json!([{ "header": "@@", "lines": [
        { "kind": "added", "old_no": null, "new_no": 1, "text": "t" }
    ]}]);
    let old = json!({
        "id": "abc", "parents": [], "author": person_json("ann"),
        "committer": person_json("bob"), "subject": "s", "body": "body",
        "files": [with_hunks, old_file], "truncated": false, "permalink": null, "link": null,
    });
    same(&typed, &old);
}

#[test]
fn tree_refs_and_repo_match_the_maps_they_replaced() {
    let tree = TreeResponse {
        commit: "c".into(),
        entries: vec![TreeItem {
            kind: TreeEntryKind::Dir,
            name: "src".into(),
            size: None,
        }],
        path: String::new(),
        rev: "HEAD".into(),
    };
    same(
        &tree,
        &json!({ "rev": "HEAD", "commit": "c", "path": "",
            "entries": [{ "name": "src", "kind": "dir", "size": null }] }),
    );
    let refs = RefsResponse {
        refs: vec![RefItem {
            kind: RefKind::Tag,
            name: "v1".into(),
            target: "t".into(),
            time: 5,
        }],
    };
    same(
        &refs,
        &json!({ "refs": [{ "name": "v1", "kind": "tag", "target": "t", "time": 5 }] }),
    );
    let repo = RepoResponse {
        head: Some("h".into()),
        head_branch: None,
        index_version: Some("\"v\"".into()),
        name: "r".into(),
        totals: RepoTotals {
            additions: 9,
            authors: 1,
            commits: 3,
            deletions: 4,
            every_commits: 5,
            files: 2,
        },
    };
    same(
        &repo,
        &json!({
            "name": "r", "head": "h", "head_branch": null, "index_version": "\"v\"",
            "totals": {
                "commits": 3, "authors": 1, "files": 2, "additions": 9, "deletions": 4,
                "every_commits": 5,
            },
        }),
    );
}

#[test]
fn blob_blame_and_contributors_match_the_maps_they_replaced() {
    let blob = BlobResponse {
        binary: false,
        commit: "c".into(),
        link: None,
        path: "a".into(),
        permalink: None,
        rev: "HEAD".into(),
        size: 7,
        text: Some("x".into()),
        total_lines: Some(1),
        truncated: false,
    };
    same(
        &blob,
        &json!({
            "rev": "HEAD", "commit": "c", "path": "a", "size": 7, "binary": false,
            "total_lines": 1, "truncated": false, "text": "x", "permalink": null, "link": null,
        }),
    );
    let blame = BlameResponse {
        commit: "c".into(),
        lines: vec![BlameItem {
            author: "ann".into(),
            commit: "c".into(),
            line_no: 1,
            text: "t".into(),
            time: 5,
        }],
        link: None,
        path: "a".into(),
        permalink: None,
        rev: "HEAD".into(),
        truncated: false,
    };
    same(
        &blame,
        &json!({
            "rev": "HEAD", "commit": "c", "path": "a", "truncated": false, "permalink": null,
            "link": null,
            "lines": [{ "line_no": 1, "commit": "c", "author": "ann", "time": 5, "text": "t" }],
        }),
    );
    let people = ContributorsResponse {
        authors: vec![ContributorItem {
            active_days: 2,
            additions: 3,
            bot: false,
            commits: 4,
            deletions: 5,
            emails: vec!["a@x.io".into()],
            first_commit: 6,
            first_time: false,
            last_commit: 7,
            name: "ann".into(),
            series: vec![1, 2],
        }],
        bus_factor: 1,
        rev: "HEAD".into(),
    };
    same(
        &people,
        &json!({ "rev": "HEAD", "bus_factor": 1, "authors": [{
            "name": "ann", "emails": ["a@x.io"], "bot": false, "commits": 4, "additions": 3,
            "deletions": 5, "first_commit": 6, "last_commit": 7, "active_days": 2,
            "series": [1, 2], "first_time": false,
        }]}),
    );
}

#[test]
fn index_and_insight_list_match_the_maps_they_replaced() {
    let row = InsightRow {
        name: "n".into(),
        summary: "s".into(),
    };
    same(
        &InsightList {
            insights: vec![row.clone()],
        },
        &json!({ "insights": [{ "name": "n", "summary": "s" }] }),
    );
    let index = IndexResponse {
        endpoints: vec![EndpointDoc {
            params: vec![ParamDoc {
                description: "d".into(),
                name: "rev".into(),
                required: false,
                ty: "string".into(),
            }],
            path: "/log".into(),
            returns: "commits".into(),
            summary: "Paged history.".into(),
        }],
        insights: vec![row],
        version: 1,
    };
    same(
        &index,
        &json!({
            "version": 1,
            "endpoints": [{ "path": "/log", "summary": "Paged history.", "returns": "commits",
                "params": [{
                    "name": "rev", "ty": "string", "required": false, "description": "d"
                }]}],
            "insights": [{ "name": "n", "summary": "s" }],
        }),
    );
}
