//! Checkpoints and viewed marks on the working tree page: the Checkpoint and Mark all viewed
//! buttons, the Since checkpoint tab with its list of unchanged files, and the three writes
//! the buttons call. The API and MCP stay read-only: these routes are the only writers, and they
//! write only the cache folder (`.git/gitterrace`).

use askama::Template;
use axum::{
    extract::Request,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use gitterrace_git::{FileDiff, Scope, SinceDiff};
use serde::Deserialize;

use crate::{
    app::App,
    error::{Error, Result},
    fmt, guard,
    pagehead::Tab,
    routes::Shared,
    when::When,
};

/// Most bytes of a write body: a mark is a path and a hash, and the store keeps 20,000 at most.
/// Not configurable: a bound on what a page may post to the local server.
const BODY_LIMIT: usize = 2 << 20;

/// What the working tree page shows of the checkpoints.
pub(crate) struct Review {
    /// The files the page lists: the tab's, or the ones that differ from the checkpoint.
    pub(crate) files: Vec<FileDiff>,
    since: Option<Since>,
    /// The newest checkpoint, which the Since checkpoint tab opens.
    latest: Option<String>,
    persisted: bool,
    /// The scan stopped at `limits.worktree_files`: the list, and a checkpoint taken from it,
    /// may be missing changed files.
    files_omitted: bool,
}

/// The checkpoint a page is compared with.
struct Since {
    id: String,
    created: i64,
    label: String,
    /// Files the checkpoint holds as they are now: the ones the API counts as unchanged.
    unchanged: Vec<String>,
    /// Files whose copy was not kept: they show against HEAD.
    gone: usize,
    /// The checkpoint's HEAD is not in the repository: the comparison may miss commits.
    baseline_missing: bool,
    /// The commits since the checkpoint could not be read.
    history_unreadable: bool,
}

fn missing(id: &str) -> Error {
    Error::Missing(format!(
        "Checkpoint {id} is not kept: the size cap may have dropped it, or checkpoints are off."
    ))
}

/// The comparison with checkpoint `id`.
fn since_diff(a: &App, id: &str) -> Result<SinceDiff> {
    let cp = a.index.checkpoint(id)?.ok_or_else(|| missing(id))?;
    Ok(gitterrace_git::worktree_diff_since(&a.repo, &cp.files, &cp.head, cp.files_omitted)?)
}

/// The files that differ from checkpoint `id`, for the folded files and find in the diff.
pub(crate) fn since_files(a: &App, id: &str) -> Result<Vec<FileDiff>> {
    Ok(since_diff(a, id)?.files)
}

/// What the page lists for `scope`, or against checkpoint `since` when one is named.
pub(crate) fn read(a: &App, scope: Scope, since: Option<&str>) -> Result<Review> {
    let list = a.api().and_then(|x| x.checkpoints().ok());
    let persisted = list.as_ref().is_some_and(|l| l.persisted);
    let latest = list.as_ref().and_then(|l| l.checkpoints.first()).map(|c| c.id.clone());
    let Some(id) = since else {
        let l = gitterrace_git::worktree_listed(&a.repo, scope)?;
        return Ok(Review {
            files: l.files,
            since: None,
            latest,
            persisted,
            files_omitted: l.files_omitted,
        });
    };
    let info = list
        .and_then(|l| l.checkpoints.into_iter().find(|c| c.id == id))
        .ok_or_else(|| missing(id))?;
    let mut d = since_diff(a, id)?;
    let unchanged = std::mem::take(&mut d.unchanged_paths);
    let since = Since {
        id: info.id,
        created: info.created,
        label: info.label,
        unchanged,
        gone: d.gone.len(),
        baseline_missing: d.baseline_missing,
        history_unreadable: d.history_unreadable,
    };
    let files_omitted = d.files_omitted;
    Ok(Review { files: d.files, since: Some(since), latest, persisted, files_omitted })
}

impl Review {
    /// Whether the scan stopped at `limits.worktree_files`: the page then names the limit.
    pub(crate) const fn files_omitted(&self) -> bool {
        self.files_omitted
    }

    /// The checkpoint id the page is compared with, if any.
    pub(crate) fn since_id(&self) -> Option<&str> {
        self.since.as_ref().map(|s| s.id.as_str())
    }

    /// `tabs` with Since checkpoint after them when one exists: on for a checkpoint page, and
    /// then no scope tab is.
    pub(crate) fn tabs(&self, mut tabs: Vec<Tab>) -> Vec<Tab> {
        let Some(id) = self.since_id().or(self.latest.as_deref()) else {
            return tabs;
        };
        let on = self.since.is_some();
        if on {
            for t in &mut tabs {
                t.on = false;
            }
        }
        let href = fmt::href(&format!("/worktree?since={}", fmt::enc_seg(id)));
        tabs.push(Tab { label: "Since checkpoint".to_owned(), href, on, zero: false });
        tabs
    }

    /// The buttons for the title row. Present only where marks and checkpoints can be kept:
    /// without them the page keeps viewed marks in the browser and `diffs-viewed.js` knows.
    pub(crate) fn bar(&self) -> String {
        Bar { persisted: self.persisted, mark_all: !self.files.is_empty() }
            .render()
            .unwrap_or_default()
    }

    /// The block of a checkpoint page: when it was taken, what its copies cover and the changed
    /// files it already holds, collapsed. Empty on any other page.
    pub(crate) fn since_note(&self) -> String {
        let Some(s) = &self.since else {
            return String::new();
        };
        SinceNote {
            when: fmt::ago(s.created, fmt::now()),
            label: s.label.clone(),
            gone: if s.gone > 0 { fmt::count(s.gone, "file", "files") } else { String::new() },
            unchanged_count: fmt::count(s.unchanged.len(), "file", "files"),
            unchanged: s.unchanged.clone(),
            baseline_missing: s.baseline_missing,
            history_unreadable: s.history_unreadable,
        }
        .render()
        .unwrap_or_default()
    }
}

#[derive(Template)]
#[template(path = "checkpoint_bar.html")]
struct Bar {
    persisted: bool,
    mark_all: bool,
}

#[derive(Template)]
#[template(path = "checkpoint_since.html")]
struct SinceNote {
    when: When,
    label: String,
    gone: String,
    unchanged_count: String,
    unchanged: Vec<String>,
    baseline_missing: bool,
    history_unreadable: bool,
}

/// Why a write is refused before its body is read: the owner proof and same-origin rules every
/// write shares, then JSON only.
pub(crate) fn refused(app: &App, req: &Request) -> Option<Response> {
    let Some(owner) = app.env.owner() else {
        return Some(
            (StatusCode::FORBIDDEN, "forbidden: no owner on this server\n").into_response(),
        );
    };
    if let Some(why) = guard::same_origin_write(req.headers(), &owner) {
        return Some((StatusCode::FORBIDDEN, why).into_response());
    }
    if !guard::is_json(req.headers()) {
        return Some((StatusCode::UNSUPPORTED_MEDIA_TYPE, "json only\n").into_response());
    }
    None
}

pub(crate) fn bad() -> Response {
    (StatusCode::BAD_REQUEST, "malformed request\n").into_response()
}

pub(crate) fn reply(v: &serde_json::Value) -> Response {
    guard::no_store(axum::Json(v).into_response())
}

/// Runs `f` on the API state off the async threads: the stores read and write files.
pub(crate) async fn with_api<T: Send + 'static>(
    app: &App,
    f: impl FnOnce(&gitterrace_api::ApiState) -> std::result::Result<T, gitterrace_api::ApiError>
    + Send
    + 'static,
) -> std::result::Result<T, Box<Response>> {
    let Some(api) = app.api().cloned() else {
        return Err(Box::new((StatusCode::SERVICE_UNAVAILABLE, "not ready\n").into_response()));
    };
    match tokio::task::spawn_blocking(move || f(&api)).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(Box::new(e.into_response())),
        Err(_) => {
            Err(Box::new((StatusCode::INTERNAL_SERVER_ERROR, "internal error\n").into_response()))
        }
    }
}

#[derive(Deserialize)]
struct CheckpointBody {
    #[serde(default)]
    scope: String,
    #[serde(default)]
    label: String,
}

/// `POST /_checkpoint` `{"scope":"all","label":""}` answers `{"id","created"}`: a checkpoint of
/// the whole working tree. The same state again answers the old id with `created: false`.
pub(crate) async fn checkpoint(axum::extract::State(app): Shared, req: Request) -> Response {
    if let Some(no) = refused(&app, &req) {
        return no;
    }
    let body = match guard::json_body::<CheckpointBody>(req, BODY_LIMIT, bad).await {
        Ok(b) => b,
        Err(res) => return *res,
    };
    if !matches!(body.scope.as_str(), "" | "all") {
        return bad();
    }
    match with_api(&app, move |api| api.checkpoint_create(&body.label, false)).await {
        Ok((id, created)) => reply(&serde_json::json!({ "id": id, "created": created })),
        Err(res) => *res,
    }
}

#[derive(Deserialize)]
struct One {
    path: String,
    patch: String,
    viewed: bool,
}

/// `GET /_viewed` answers the marks as `GET /api/v1/viewed` does, but on the page's own route:
/// the page reads its marks here, so they work with `features.api_enabled` off.
pub(crate) async fn viewed_list(axum::extract::State(app): Shared) -> Response {
    match with_api(&app, gitterrace_api::ApiState::viewed).await {
        Ok(v) => reply(&serde_json::to_value(&v).unwrap_or_default()),
        Err(res) => *res,
    }
}

/// `POST /_viewed` `{"path","patch","viewed"}`: marks one file viewed at `patch`, or clears it.
pub(crate) async fn viewed(axum::extract::State(app): Shared, req: Request) -> Response {
    if let Some(no) = refused(&app, &req) {
        return no;
    }
    let one = match guard::json_body::<One>(req, BODY_LIMIT, bad).await {
        Ok(b) => b,
        Err(res) => return *res,
    };
    if one.path.is_empty() {
        return bad();
    }
    let items = vec![(one.path, one.patch)];
    match with_api(&app, move |api| api.viewed_set(&items, one.viewed)).await {
        Ok(()) => reply(&serde_json::json!({ "ok": true })),
        Err(res) => *res,
    }
}

#[derive(Deserialize)]
struct Item {
    path: String,
    patch: String,
}

#[derive(Deserialize)]
struct Many {
    items: Vec<Item>,
    /// Mark all viewed asks for a checkpoint first. It is taken only when
    /// `review.has_auto_checkpoint` is on, so the page need not know the setting.
    #[serde(default)]
    checkpoint: bool,
    /// Marks only the paths that have no record yet, an unmark included: the browser's move of
    /// its old marks must not undo one another tab or a later click saved.
    #[serde(default)]
    missing_only: bool,
}

/// `POST /_viewed/all` `{"items":[{"path","patch"}],"checkpoint":false}`: marks every item
/// viewed, after a checkpoint when asked and `review.has_auto_checkpoint` is on. Answers
/// `{"marked", "checkpoint"}`, the id of the checkpoint taken or null.
pub(crate) async fn viewed_all(axum::extract::State(app): Shared, req: Request) -> Response {
    if let Some(no) = refused(&app, &req) {
        return no;
    }
    let many = match guard::json_body::<Many>(req, BODY_LIMIT, bad).await {
        Ok(b) => b,
        Err(res) => return *res,
    };
    if many.items.iter().any(|i| i.path.is_empty()) {
        return bad();
    }
    let items: Vec<(String, String)> = many.items.into_iter().map(|i| (i.path, i.patch)).collect();
    let want = many.checkpoint;
    let missing_only = many.missing_only;
    let done = with_api(&app, move |api| {
        if missing_only {
            return Ok((api.viewed_add_missing(&items)?, None));
        }
        let id = if want && api.has_auto_checkpoint() {
            Some(api.checkpoint_create("", true)?.0)
        } else {
            None
        };
        api.viewed_set(&items, true)?;
        Ok((items.len(), id))
    })
    .await;
    match done {
        Ok((n, id)) => reply(&serde_json::json!({ "marked": n, "checkpoint": id })),
        Err(res) => *res,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use gitterrace_git::{
        Repo, Scope, worktree_diff_scoped, worktree_diff_since, worktree_snapshot,
    };
    use gitterrace_testkit::{Fixture, git};

    /// Guard class: the unchanged list of the page is the one it always had (changed files
    /// against HEAD that the checkpoint already holds as they are), now read off the single
    /// comparison. Held, not-held, changed, gone and saved-then-committed files all agree with
    /// the old second scan, so a drift in either side fails here.
    #[test]
    fn unchanged_list_equals_the_old_scan_for_every_kind_of_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let p = dir.path();
        let _ = Fixture::init(p)
            .write("held.txt", "1\n")
            .write("moved.txt", "1\n")
            .write("clean.txt", "1\n")
            .write("later.txt", "1\n")
            .write("saved.txt", "1\n")
            .write("x.dat", "1\n")
            .commit("init");
        let base = git(p, &["rev-parse", "HEAD"]).trim().to_owned();
        std::fs::write(p.join("held.txt"), "held\n").expect("w");
        std::fs::write(p.join("moved.txt"), "a\n").expect("w");
        std::fs::write(p.join("saved.txt"), "saved\n").expect("w");
        std::fs::write(p.join("x.dat"), b"\0\x01").expect("w");
        let repo = Repo::discover(p).expect("repo");
        let snap = worktree_snapshot(&repo, 1 << 20, u64::MAX).expect("snap");
        // The checkpoint keeps no copy of x.dat: it is "gone" once the file moves on.
        let held: Vec<_> = snap
            .files
            .into_iter()
            .map(|mut f| {
                if f.path == "x.dat" {
                    f.text = None;
                }
                f
            })
            .collect();
        // later.txt: committed after the checkpoint, then put back to the old text.
        std::fs::write(p.join("later.txt"), "2\n").expect("w");
        git(p, &["commit", "-qm", "later", "--", "later.txt"]);
        std::fs::write(p.join("later.txt"), "1\n").expect("w");
        // saved.txt: committed exactly as the checkpoint saved it.
        git(p, &["commit", "-qm", "saved", "--", "saved.txt"]);
        std::fs::write(p.join("moved.txt"), "b\n").expect("w");
        std::fs::write(p.join("x.dat"), b"\0\x02").expect("w");
        let d = worktree_diff_since(&repo, &held, &base, false).expect("since");
        let moved: HashSet<String> = d.files.iter().map(|f| f.path.clone()).collect();
        let old: Vec<String> = worktree_diff_scoped(&repo, Scope::All)
            .expect("scan")
            .into_iter()
            .map(|f| f.path)
            .filter(|p| !moved.contains(p))
            .collect();
        let mut old_sorted = old;
        old_sorted.sort();
        assert!(old_sorted.iter().all(|p| d.unchanged_paths.contains(p)));
        assert!(d.unchanged_paths.contains(&"held.txt".to_owned()), "{:?}", d.unchanged_paths);
        assert!(d.unchanged_paths.contains(&"later.txt".to_owned()), "not held, equal to base");
        assert!(
            d.unchanged_paths.contains(&"saved.txt".to_owned()),
            "saved then committed as saved: still unchanged since the checkpoint"
        );
        assert_eq!(d.unchanged_paths.len(), d.unchanged, "the page lists what the API counts");
        assert!(moved.contains("moved.txt") && d.gone == ["x.dat"]);
    }
}
