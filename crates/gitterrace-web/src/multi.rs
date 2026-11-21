//! Several repositories on one server: a name table, one lazily built router per repository
//! under `/r/NAME`, and a repository index at `/`. The table starts with the CLI paths, grows
//! and shrinks only through the local secret route (see `add.rs`), and never empties. Every
//! repository lives under `/r/NAME`, also alone: a link handed out never changes when another
//! repository joins or leaves. `/` forwards to the one repository while there is one. A name is
//! assigned once and kept in the registry (see `registry::claim`).

mod discover;
mod home;
mod recent;
pub(crate) mod sweep;

use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use gitterrace_git::Repo;
use tokio::sync::OnceCell;
use tower::ServiceExt;

use crate::app::{App, Env};
use crate::fmt;
pub use crate::multi::discover::{discover_repos, search_roots};
use crate::multi::recent::Recent;

pub(crate) struct Served {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) root: PathBuf,
    router: OnceCell<Router>,
    /// Set for an addition until its router is built: the repository is listed as indexing and
    /// its pages answer 503 meanwhile, instead of waiting on a cache another process may hold.
    opening: AtomicBool,
    /// The background tasks the router started, stopped when the repository is taken off.
    /// `None` once stopped: a router built after that (its build was already under way) has
    /// its tasks aborted at once instead of stored, so they never outlive the removal.
    tasks: Mutex<Option<Vec<tokio::task::AbortHandle>>>,
    app: Mutex<Weak<App>>,
    /// Newest commit and the sparkline's counts at the HEAD they were read for, so the index page
    /// opens a repository once per HEAD move or new local day instead of once per hit.
    recent: Mutex<Option<(String, Arc<Recent>)>>,
    /// The working tree's dirty answer and when it was measured, reused for `DIRTY_TTL`.
    dirty: crate::app::DirtyMemo,
}

pub(crate) type Built = (Router, Vec<tokio::task::AbortHandle>, Weak<App>);

/// The repository table. A URL name resolves only through this map, never by joining it
/// onto a filesystem path.
pub(crate) struct Table {
    env: Env,
    repos: RwLock<Repos>,
    /// The registry folder names are kept in; `None` names from the table alone.
    state: Option<PathBuf>,
    /// Actions, when on; each repository's pages embed its token.
    pub(crate) live: Option<Arc<crate::actions::Live>>,
}

#[derive(Default)]
struct Repos {
    order: Vec<Arc<Served>>,
    by_name: HashMap<String, Arc<Served>>,
    /// The same names for the related links, kept in step by `insert` and `drop_name`.
    peers: crate::related::Peers,
}

/// Why [`Table::take`] left the table as it was.
/// A folder name that cannot be a URL name; holds the name.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("unusable repository name {0:?}")]
pub(crate) struct NameError(pub(crate) String);

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Kept {
    NotServed,
    Last,
}

/// What [`Table::add`] did: the name the repository is served under, and whether it was new.
pub(crate) enum Added {
    Fresh(Arc<Served>),
    Present { name: String, base: String },
}

#[derive(serde::Serialize)]
struct RepoItem {
    name: String,
    href: String,
    when: crate::when::When,
}

impl Table {
    /// Names each repository after its folder (see [`Self::allocate`]). Two paths of one
    /// repository are served once.
    pub(crate) fn new(paths: Vec<PathBuf>, env: Env, state: Option<PathBuf>) -> io::Result<Self> {
        let table = Self {
            repos: RwLock::new(Repos { peers: env.related.clone(), ..Repos::default() }),
            env,
            state,
            live: None,
        };
        for path in paths {
            let repo = Repo::discover(&path).map_err(io::Error::other)?;
            let root = repo.root();
            let mut g = table.write();
            if g.order.iter().any(|s| s.root == root) {
                continue;
            }
            let name =
                table.allocate(&g, &url_stem(&repo.name()), &root).map_err(|NameError(stem)| {
                    io::Error::other(format!(
                        "unusable repository name {stem:?} for {}",
                        path.display()
                    ))
                })?;
            g.insert(name, path, root, false);
        }
        Ok(table)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Repos> {
        self.repos.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Repos> {
        self.repos.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The URL name of the repository at `root`: the one the registry keeps for it, else a
    /// new one, first come first served (`registry::claim`). Without a registry, or when it
    /// cannot be read or written, the same rule against the table alone, so a server always
    /// starts. The stem back when it is not URL-safe.
    fn allocate(&self, repos: &Repos, stem: &str, root: &Path) -> Result<String, NameError> {
        if !valid_name(stem) {
            return Err(NameError(stem.to_owned()));
        }
        let taken = |n: &str| repos.order.iter().any(|s| s.name.eq_ignore_ascii_case(n));
        if let Some(dir) = &self.state {
            match crate::registry::claim(dir, root, stem, &taken) {
                // A recorded name is served only when no repository here holds it already.
                Ok(name) if valid_name(&name) && !taken(&name) => return Ok(name),
                Ok(name) => tracing::warn!("recorded name {name:?} unusable here"),
                Err(e) => tracing::warn!("naming {}: {e}", root.display()),
            }
        }
        Ok(crate::registry::pick(stem, root, &taken))
    }

    /// Adds the repository whose canonical top folder is `root`, named like the startup ones.
    /// A root already served comes back as [`Added::Present`]; check and insert hold one lock,
    /// so two racing additions of the same folder add it once.
    pub(crate) fn add(&self, root: PathBuf, stem: &str) -> Result<Added, NameError> {
        let mut g = self.write();
        if let Some(s) = g.order.iter().find(|s| s.root == root) {
            return Ok(Added::Present { name: s.name.clone(), base: format!("/r/{}/", s.name) });
        }
        let name = self.allocate(&g, &url_stem(stem), &root)?;
        Ok(Added::Fresh(g.insert(name, root.clone(), root, true)))
    }

    /// Drops `served` again, for an addition whose repository would not open. Its name claim
    /// goes too, so it is not held for a repository that was never served. Only this very
    /// entry: the name may belong to a repository added again meanwhile, which stays. Whether
    /// it was still served.
    pub(crate) fn remove(&self, served: &Arc<Served>) -> bool {
        let mut g = self.write();
        if !g.by_name.get(&served.name).is_some_and(|s| Arc::ptr_eq(s, served)) {
            return false;
        }
        let gone = g.drop_name(&served.name);
        drop(g);
        if let (Some(dir), Some(s)) = (&self.state, gone)
            && let Err(e) = crate::registry::forget(dir, &s.root)
        {
            tracing::warn!("forgetting the name {}: {e}", served.name);
        }
        true
    }

    /// Takes `name` off the running server, as [`Self::take`] does: its URLs answer 404. For a
    /// repository whose folder or git directory is gone from disk, so the last one goes too: a
    /// ghost would answer an empty page, and the next sweep brings it back when it returns.
    /// False when it was not served.
    pub(crate) fn drop_gone(&self, name: &str) -> bool {
        let gone = self.write().drop_name(name);
        gone.inspect(|s| s.stop()).is_some()
    }

    /// Takes the repository named `name`, or at canonical `root`, off the running server: its
    /// URLs answer 404 from now on, and its index refresher, idle release and warm-up stop, so
    /// its caches go with the last in-flight request. An index update already running stops
    /// before its next write and its index child is killed and reaped before this returns, so
    /// the cache keeps its last whole set and nothing writes into it after. The on-disk index
    /// and the name claim stay, so adding it again is fast and keeps its URL. The last
    /// repository stays.
    pub(crate) fn take(&self, name: Option<&str>, root: Option<&Path>) -> Result<String, Kept> {
        let mut g = self.write();
        let found = g
            .order
            .iter()
            .find(|s| {
                name == Some(s.name.as_str())
                    || root
                        .is_some_and(|r| s.root == r || s.root.canonicalize().is_ok_and(|c| c == r))
            })
            .map(|s| s.name.clone())
            .ok_or(Kept::NotServed)?;
        if g.order.len() == 1 {
            return Err(Kept::Last);
        }
        let gone = g.drop_name(&found);
        drop(g);
        if let Some(s) = gone {
            s.stop();
        }
        Ok(found)
    }

    /// The page for a repository that will not open. An io error names host folders, so only
    /// a local server shows it; a public one says what failed and nothing more.
    pub(crate) fn open_failed(&self, name: &str, e: &io::Error) -> Response {
        let detail = if self.env.public() {
            format!("The repository {name} could not be opened.")
        } else {
            format!("The repository {name} could not be opened: {e}")
        };
        let status = StatusCode::INTERNAL_SERVER_ERROR;
        (status, crate::error::page(status, &detail)).into_response()
    }

    pub(crate) fn get(&self, name: &str) -> Option<Arc<Served>> {
        self.read().by_name.get(name).cloned()
    }

    /// Builds every repository's router now, in the background: its index refresher and
    /// warm-up start at once, as they did for a repository served at `/`, not on the first visit.
    pub(crate) fn start_all(self: &Arc<Self>) {
        for served in self.snapshot() {
            let table = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(e) = table.router_of(&served).await {
                    tracing::warn!("opening {}: {e}", served.name);
                }
            });
        }
    }

    /// The served repositories in order, a copy so no lock is held while one is read.
    fn snapshot(&self) -> Vec<Arc<Served>> {
        self.read().order.clone()
    }

    /// The repository's router, built on first use. Building starts its index refresher and
    /// warm-up, so an addition calls it at once rather than on the first visit. The build runs
    /// as its own task on the blocking pool (it opens the cache, which can wait for another
    /// writer) and is never cut short by the caller going away: a router built and then
    /// dropped would leave its tasks running for no one.
    pub(crate) async fn router_of(self: &Arc<Self>, served: &Arc<Served>) -> io::Result<Router> {
        let (table, served) = (Arc::clone(self), Arc::clone(served));
        tokio::spawn(async move { table.build(&served).await }).await.map_err(io::Error::other)?
    }

    async fn build(&self, served: &Arc<Served>) -> io::Result<Router> {
        served
            .router
            .get_or_try_init(|| async {
                let (s, live, env) = (Arc::clone(served), self.live.clone(), self.env.clone());
                let (router, tasks, app) =
                    tokio::task::spawn_blocking(move || repo_router(&s, live, &env))
                        .await
                        .map_err(io::Error::other)??;
                served.adopt(tasks, &app);
                served.opening.store(false, Ordering::Release);
                Ok::<_, io::Error>(router)
            })
            .await
            .cloned()
    }

    /// Tells every open tab, of every repository, that the table changed.
    pub(crate) fn announce(&self) {
        self.env.repos_changed();
    }

    /// Each repository opened once, keyed by its URL name, for permalinks and run requests.
    pub(crate) fn targets(&self) -> io::Result<Vec<crate::actions::Target>> {
        self.snapshot()
            .iter()
            .map(|s| {
                Ok(crate::actions::Target {
                    key: s.name.clone(),
                    base: format!("/r/{}", s.name),
                    repo: Repo::discover(&s.path).map_err(io::Error::other)?,
                })
            })
            .collect()
    }
}

impl Repos {
    fn insert(&mut self, name: String, path: PathBuf, root: PathBuf, opening: bool) -> Arc<Served> {
        let served = Arc::new(Served {
            name: name.clone(),
            path,
            root,
            router: OnceCell::new(),
            opening: AtomicBool::new(opening),
            tasks: Mutex::new(Some(Vec::new())),
            app: Mutex::new(Weak::new()),
            recent: Mutex::new(None),
            dirty: crate::app::DirtyMemo::default(),
        });
        self.peers.add(&name, served.path.clone());
        self.by_name.insert(name, Arc::clone(&served));
        self.order.push(Arc::clone(&served));
        served
    }

    fn drop_name(&mut self, name: &str) -> Option<Arc<Served>> {
        self.order.retain(|s| s.name != name);
        self.peers.remove(name);
        self.by_name.remove(name)
    }
}

impl Served {
    /// Stops the background tasks its router started and every index write: an update
    /// running in-process returns at its next record, the index child is killed and reaped
    /// before this returns.
    pub(crate) fn stop(&self) {
        let held = self.tasks.lock().map(|mut g| g.take()).unwrap_or_default();
        for t in held.unwrap_or_default() {
            t.abort();
        }
        let app = self.app.lock().map(|g| g.upgrade()).unwrap_or_default();
        if let Some(app) = app {
            app.halt();
        }
    }

    /// Keeps the tasks and app a fresh router started, or aborts and halts them when the
    /// repository was taken off while its router was being built.
    fn adopt(&self, tasks: Vec<tokio::task::AbortHandle>, app: &Weak<App>) {
        let mut late = tasks;
        let mut stopped = true;
        if let Ok(mut g) = self.tasks.lock()
            && let Some(held) = g.as_mut()
        {
            held.append(&mut late);
            stopped = false;
            if let Ok(mut a) = self.app.lock() {
                *a = app.clone();
            }
        }
        if stopped && let Some(a) = app.upgrade() {
            a.halt();
        }
        for t in late {
            t.abort();
        }
    }

    /// Whether the working tree differs from HEAD, reused for `DIRTY_TTL` like a repository's
    /// own chip: the home page then scans each tree at most once per bound, however often it
    /// is reloaded, and a change shows within the bound.
    fn dirty(&self) -> bool {
        if let Some(v) = self.dirty.get() {
            return v;
        }
        let v = Repo::discover(&self.path).is_ok_and(|r| gitterrace_git::is_dirty(&r));
        self.dirty.put(v);
        v
    }

    /// The insights state of its app, `None` until its router is built: a repository is
    /// served at once, its app only on first use or an addition.
    fn insights_state(&self) -> Option<crate::app::RepoState> {
        let built = self.app.lock().ok()?.upgrade().map(|a| a.state());
        built.or_else(|| {
            self.opening.load(Ordering::Acquire).then_some(crate::app::RepoState::Indexing)
        })
    }

    fn opening(&self) -> bool {
        self.opening.load(Ordering::Acquire) && self.router.get().is_none()
    }
}

/// The folder name made URL-safe: lowercase ASCII, anything else (a dot prefix, spaces,
/// separators, control or non-ASCII characters) a `-`, trimmed of `-`; `repo` when nothing is
/// left. Lowercase, so two folders differing in case only never share a URL on a
/// case-insensitive disk.
pub(crate) fn url_stem(folder: &str) -> String {
    // A bare clone's folder is conventionally `name.git`: the repository is `name`.
    let folder = folder.strip_suffix(".git").filter(|f| !f.is_empty()).unwrap_or(folder);
    let mapped: String = folder
        .trim_start_matches('.')
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' | '_' | '.' => c,
            'A'..='Z' => c.to_ascii_lowercase(),
            _ => '-',
        })
        .collect();
    let trimmed = mapped.trim_matches(['-', '.']);
    if trimmed.is_empty() { "repo".to_owned() } else { trimmed.to_owned() }
}

/// A URL-safe name: no separator, no dot prefix (so never `.` or `..`), nothing to escape.
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Index at `/`, the switcher list, shared assets, and every repository under `/r/NAME`.
pub(crate) fn router(table: Arc<Table>) -> Router {
    Router::new()
        .route("/", get(home::index))
        .route("/api/repos", get(repos_json))
        .route("/_dirty", get(home::dirty))
        .route("/events", get(home_events))
        .route("/r/{name}", get(slash))
        .route("/r/{name}/", axum::routing::any(dispatch))
        .route("/r/{name}/{*rest}", axum::routing::any(dispatch))
        .merge(crate::routes::assets())
        .fallback(crate::error::fallback)
        .with_state(table)
}

/// `/events` beside the home: the repository list's live stream, see `live::home_events`.
async fn home_events(
    State(table): State<Arc<Table>>,
    axum::extract::Query(q): axum::extract::Query<crate::live::EventsQuery>,
) -> Response {
    crate::live::home_events(&table.env, &q)
}

async fn slash(uri: Uri) -> Redirect {
    let q = uri.query().map_or_else(String::new, |q| format!("?{q}"));
    Redirect::permanent(&format!("{}/{q}", uri.path()))
}

/// Hands the request to the repository's own router with the `/r/NAME` prefix removed.
async fn dispatch(State(table): State<Arc<Table>>, req: Request) -> Response {
    let raw = req.uri().path().to_owned();
    let Some(after) = raw.strip_prefix("/r/") else {
        return crate::error::not_found("Nothing here.");
    };
    let (seg, rest) = after.split_once('/').unwrap_or((after, ""));
    // The raw segment must match a table name byte for byte: an encoded `%2e%2e` or `%2F`
    // never equals a valid name, so it falls through to 404.
    let Some(served) = table.get(seg) else {
        return crate::error::not_found(&format!(
            "No repository named {seg} is served here. The overview lists the ones that are."
        ));
    };
    if served.opening() {
        return opening_page(&served.name);
    }
    let router = match table.router_of(&served).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("opening {}: {e}", served.name);
            return table.open_failed(&served.name, &e);
        }
    };
    let pq = req.uri().query().map_or_else(|| format!("/{rest}"), |q| format!("/{rest}?{q}"));
    let (parts, body) = req.into_parts();
    let Ok(uri) = pq.parse::<Uri>() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    // A fresh request: the outer route's extensions (its `{name}`/`{*rest}` params) would
    // otherwise be read by the inner router's `Path` extractors.
    let mut inner = Request::new(body);
    *inner.method_mut() = parts.method;
    *inner.uri_mut() = uri;
    *inner.version_mut() = parts.version;
    *inner.headers_mut() = parts.headers;
    match router.oneshot(inner).await {
        Ok(r) => r,
        Err(never) => match never {},
    }
}

/// What a page of an added repository answers while it opens: it is listed and indexing, and
/// the page can be asked again.
fn opening_page(name: &str) -> Response {
    let status = StatusCode::SERVICE_UNAVAILABLE;
    let detail = format!("The repository {name} is being added and indexed. Try again shortly.");
    (status, [(axum::http::header::RETRY_AFTER, "2")], crate::error::page(status, &detail))
        .into_response()
}

fn repo_router(
    served: &Served,
    live: Option<Arc<crate::actions::Live>>,
    env: &Env,
) -> io::Result<Built> {
    let base = format!("/r/{}", served.name);
    crate::repo_router_tasks(&served.path, base, served.name.clone(), live, env)
}

async fn repos_json(State(table): State<Arc<Table>>) -> Response {
    let items = tokio::task::spawn_blocking(move || {
        let now = fmt::now();
        table
            .snapshot()
            .iter()
            .map(|s| RepoItem {
                name: s.name.clone(),
                href: format!("/r/{}/", s.name),
                when: s
                    .recent(now)
                    .last
                    .as_ref()
                    .map_or_else(crate::when::When::none, |c| fmt::ago(c.author.time, now)),
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    axum::Json(items).into_response()
}

#[cfg(test)]
mod tests;
