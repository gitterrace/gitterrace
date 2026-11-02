//! Several repositories on one server: a name table, one lazily built router per repository
//! under `/r/NAME`, and a repository index at `/`. The table starts with the CLI paths, grows
//! and shrinks only through the local secret route (see `add.rs`), and never empties. Every
//! repository lives under `/r/NAME`, also alone: a link handed out never changes when another
//! repository joins or leaves. `/` forwards to the one repository while there is one. A name is
//! assigned once and kept in the registry (see `registry::claim`).

mod discover;

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
};

use askama::Template;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use gitterrace_git::{CommitSummary, Repo};
use tokio::sync::OnceCell;
use tower::ServiceExt;

use crate::app::Env;
use crate::fmt;
pub use crate::multi::discover::discover_repos;

/// Weeks shown in the index sparkline.
const SPARK_WEEKS: usize = 26;
const WEEK: i64 = 7 * 86_400;

/// One served repository: its URL name and where it lives.
pub(crate) struct Served {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    /// Canonical top folder, what an addition is compared against.
    pub(crate) root: PathBuf,
    router: OnceCell<Router>,
    /// The background tasks the router started, stopped when the repository is taken off.
    tasks: Mutex<Vec<tokio::task::AbortHandle>>,
    /// Newest commit and weekly commit counts at the HEAD they were read for, so the index page
    /// opens a repository once per HEAD move instead of once per hit.
    recent: Mutex<Option<(String, Arc<Recent>)>>,
}

/// What the index shows for one repository: its newest commit and commits per week.
#[derive(Default)]
struct Recent {
    last: Option<CommitSummary>,
    /// Oldest week first, `SPARK_WEEKS` long; the last entry is the current week.
    weeks: Vec<u32>,
}

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
}

/// Why [`Table::take`] left the table as it was.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Kept {
    NotServed,
    Last,
}

/// What [`Table::add`] did: the name the repository is served under, and whether it was new.
pub(crate) enum Added {
    Fresh(Arc<Served>),
    /// Already served: under this URL prefix.
    Present {
        name: String,
        base: String,
    },
}

#[derive(Template)]
#[template(path = "repos.html")]
struct IndexPage {
    rows: Vec<IndexRow>,
}

struct IndexRow {
    name: String,
    href: String,
    subject: String,
    when: String,
    commit_href: String,
    spark: String,
    /// Commits in the sparkline's window, and its busiest week (the line's top).
    total: u32,
    tip: String,
}

#[derive(serde::Serialize)]
struct RepoItem {
    name: String,
    href: String,
    when: String,
}

impl Table {
    /// Names each repository after its folder (see [`Self::allocate`]). Two paths of one
    /// repository are served once.
    pub(crate) fn new(paths: Vec<PathBuf>, env: Env, state: Option<PathBuf>) -> io::Result<Self> {
        let table = Self {
            env,
            repos: RwLock::default(),
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
            let name = table
                .allocate(&g, &url_stem(&repo.name()), &root)
                .map_err(|stem| {
                    io::Error::other(format!(
                        "unusable repository name {stem:?} for {}",
                        path.display()
                    ))
                })?;
            g.insert(name, path, root);
        }
        Ok(table)
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Repos> {
        self.repos
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Repos> {
        self.repos
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The URL name of the repository at `root`: the one the registry keeps for it, else a
    /// new one, first come first served (`registry::claim`). Without a registry, or when it
    /// cannot be read or written, the same rule against the table alone, so a server always
    /// starts. The stem back when it is not URL-safe.
    fn allocate(&self, repos: &Repos, stem: &str, root: &Path) -> Result<String, String> {
        if !valid_name(stem) {
            return Err(stem.to_owned());
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
    pub(crate) fn add(&self, root: PathBuf, stem: &str) -> Result<Added, String> {
        let mut g = self.write();
        if let Some(s) = g.order.iter().find(|s| s.root == root) {
            return Ok(Added::Present {
                name: s.name.clone(),
                base: format!("/r/{}/", s.name),
            });
        }
        let name = self.allocate(&g, &url_stem(stem), &root)?;
        Ok(Added::Fresh(g.insert(name, root.clone(), root)))
    }

    /// Drops `name` again, for an addition whose repository would not open.
    /// Its name claim goes too, so it is not held for a repository that was never served.
    pub(crate) fn remove(&self, name: &str) {
        let gone = self.write().drop_name(name);
        if let (Some(dir), Some(s)) = (&self.state, gone)
            && let Err(e) = crate::registry::forget(dir, &s.root)
        {
            tracing::warn!("forgetting the name {name}: {e}");
        }
    }

    /// Takes the repository named `name`, or at canonical `root`, off the running server: its
    /// URLs answer 404 from now on, and its index refresher, idle release and warm-up stop, so
    /// its caches go with the last in-flight request. An index update already running in a
    /// blocking thread finishes its batch first; nothing starts after. The on-disk index and the
    /// name claim stay, so adding it again is fast and keeps its URL. The last repository stays.
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

    /// The URL name the repository at `root` is served under.
    #[cfg(test)]
    pub(crate) fn name_of(&self, root: &Path) -> Option<String> {
        let g = self.read();
        g.order
            .iter()
            .find(|s| s.root == root)
            .map(|s| s.name.clone())
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
    /// warm-up, so an addition calls it at once rather than on the first visit.
    pub(crate) async fn router_of(&self, served: &Served) -> io::Result<Router> {
        served
            .router
            .get_or_try_init(|| async {
                let (router, tasks) = repo_router(served, self.live.clone(), &self.env)?;
                if let Ok(mut g) = served.tasks.lock() {
                    *g = tasks;
                }
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
    fn insert(&mut self, name: String, path: PathBuf, root: PathBuf) -> Arc<Served> {
        let served = Arc::new(Served {
            name: name.clone(),
            path,
            root,
            router: OnceCell::new(),
            tasks: Mutex::default(),
            recent: Mutex::new(None),
        });
        self.by_name.insert(name, Arc::clone(&served));
        self.order.push(Arc::clone(&served));
        served
    }

    fn drop_name(&mut self, name: &str) -> Option<Arc<Served>> {
        self.order.retain(|s| s.name != name);
        self.by_name.remove(name)
    }
}

impl Served {
    /// Stops the background tasks its router started.
    fn stop(&self) {
        if let Ok(mut g) = self.tasks.lock() {
            for t in g.drain(..) {
                t.abort();
            }
        }
    }

    /// The newest commit and the weekly counts, one discover and one bounded walk per HEAD.
    /// The walk stops at the oldest week shown, so it counts every commit in the window
    /// however busy the repository is.
    fn recent(&self, now: i64) -> Arc<Recent> {
        let Ok(repo) = Repo::discover(&self.path) else {
            return Arc::default();
        };
        let Ok(head) = repo.resolve("HEAD") else {
            return Arc::default();
        };
        if let Ok(g) = self.recent.lock()
            && let Some((at, r)) = g.as_ref()
            && *at == head
        {
            return Arc::clone(r);
        }
        let times = repo
            .commit_times_since(&head, now - WEEK * SPARK_WEEKS as i64)
            .unwrap_or_default();
        let r = Arc::new(Recent {
            last: repo
                .log(&head, None, 0, 1)
                .ok()
                .and_then(|l| l.into_iter().next()),
            weeks: weekly(&times, now),
        });
        if let Ok(mut g) = self.recent.lock() {
            *g = Some((head, Arc::clone(&r)));
        }
        r
    }
}

/// The folder name made URL-safe: lowercase ASCII, anything else (a dot prefix, spaces,
/// separators, control or non-ASCII characters) a `-`, trimmed of `-`; `repo` when nothing is
/// left. Lowercase, so two folders differing in case only never share a URL on a
/// case-insensitive disk.
fn url_stem(folder: &str) -> String {
    // A bare clone's folder is conventionally `name.git`: the repository is `name`.
    let folder = folder
        .strip_suffix(".git")
        .filter(|f| !f.is_empty())
        .unwrap_or(folder);
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
    if trimmed.is_empty() {
        "repo".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// A URL-safe name: no separator, no dot prefix (so never `.` or `..`), nothing to escape.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Index at `/`, the switcher list, shared assets, and every repository under `/r/NAME`.
pub(crate) fn router(table: Arc<Table>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/repos", get(repos_json))
        .route("/r/{name}", get(slash))
        .route("/r/{name}/", axum::routing::any(dispatch))
        .route("/r/{name}/{*rest}", axum::routing::any(dispatch))
        .merge(crate::routes::assets())
        .fallback(crate::error::fallback)
        .with_state(table)
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
    let router = match table.router_of(&served).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("opening {}: {e}", served.name);
            return table.open_failed(&served.name, &e);
        }
    };
    let pq = req
        .uri()
        .query()
        .map_or_else(|| format!("/{rest}"), |q| format!("/{rest}?{q}"));
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

fn repo_router(
    served: &Served,
    live: Option<Arc<crate::actions::Live>>,
    env: &Env,
) -> io::Result<(Router, Vec<tokio::task::AbortHandle>)> {
    let base = format!("/r/{}", served.name);
    crate::repo_router_tasks(&served.path, base, served.name.clone(), live, env)
}

/// Counts `times` (seconds) into `SPARK_WEEKS` buckets ending at `now`, oldest first.
fn weekly(times: &[i64], now: i64) -> Vec<u32> {
    let mut weeks = vec![0u32; SPARK_WEEKS];
    for t in times {
        let age = (now - t).max(0) / WEEK;
        if let Ok(i) = usize::try_from(age)
            && i < SPARK_WEEKS
        {
            weeks[SPARK_WEEKS - 1 - i] += 1;
        }
    }
    weeks
}

/// Weekly counts as a polyline in a 90x20 box, scaled to `max` so rows share one scale.
fn spark(weeks: &[u32], max: u32) -> String {
    let max = max.max(1);
    let mut pts = String::new();
    for (i, n) in weeks.iter().enumerate() {
        let x = i as f64 * 90.0 / (SPARK_WEEKS - 1) as f64;
        let y = 19.0 - f64::from(*n) * 18.0 / f64::from(max);
        let _ = write!(pts, "{x:.1},{y:.1} ");
    }
    pts.trim_end().to_owned()
}

/// `/`: the one repository while there is one (a 302, so the answer changes once another
/// joins), else the index of all.
async fn index(State(table): State<Arc<Table>>) -> crate::error::Result<Response> {
    if let [one] = table.snapshot().as_slice() {
        let to = format!("/r/{}/", one.name);
        return Ok((StatusCode::FOUND, [(axum::http::header::LOCATION, to)]).into_response());
    }
    let html = tokio::task::spawn_blocking(move || {
        let now = fmt::now();
        let rows = table
            .snapshot()
            .iter()
            .map(|s| {
                let href = format!("/r/{}/", s.name);
                let r = s.recent(now);
                let c = r.last.as_ref();
                let peak = r.weeks.iter().copied().max().unwrap_or(0);
                IndexRow {
                    name: s.name.clone(),
                    subject: c.map(|c| c.subject.clone()).unwrap_or_default(),
                    when: c.map(|c| fmt::ago(c.author.time, now)).unwrap_or_default(),
                    commit_href: c.map_or_else(
                        || href.clone(),
                        |c| format!("/r/{}/commit/{}", s.name, c.id),
                    ),
                    spark: spark(&r.weeks, peak),
                    total: r.weeks.iter().sum(),
                    tip: format!(
                        "{} in 26 weeks, busiest week {peak}.",
                        fmt::count(r.weeks.iter().sum::<u32>(), "commit", "commits")
                    ),
                    href,
                }
            })
            .collect();
        IndexPage { rows }.render()
    })
    .await??;
    Ok(Html(html).into_response())
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
                    .map(|c| fmt::ago(c.author.time, now))
                    .unwrap_or_default(),
            })
            .collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    axum::Json(items).into_response()
}

#[cfg(test)]
mod tests;
