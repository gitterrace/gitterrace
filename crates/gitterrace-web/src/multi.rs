//! Several repositories on one server: a name table fixed at startup, one lazily built
//! router per repository under `/r/NAME`, and a repository index at `/`.

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
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

/// Weeks shown in the index sparkline.
const SPARK_WEEKS: usize = 26;
const WEEK: i64 = 7 * 86_400;

/// One served repository: its URL name and where it lives.
pub(crate) struct Served {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    router: OnceCell<Router>,
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

/// The startup repository table. A URL name resolves only through this map, never by
/// joining it onto a filesystem path.
pub(crate) struct Table {
    env: Env,
    order: Vec<Arc<Served>>,
    by_name: HashMap<String, Arc<Served>>,
    /// Actions, when on; each repository's pages embed its token.
    pub(crate) live: Option<Arc<crate::actions::Live>>,
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
    peak: u32,
}

#[derive(serde::Serialize)]
struct RepoItem {
    name: String,
    href: String,
    when: String,
}

impl Table {
    /// Names each repository after its folder, deduplicated with a numeric suffix.
    pub(crate) fn new(paths: Vec<PathBuf>, env: Env) -> io::Result<Self> {
        let mut order = Vec::with_capacity(paths.len());
        let mut by_name = HashMap::new();
        for path in paths {
            let stem = Repo::discover(&path).map_err(io::Error::other)?.name();
            if !valid_name(&stem) {
                return Err(io::Error::other(format!(
                    "unusable repository name {stem:?} for {}",
                    path.display()
                )));
            }
            let mut name = stem.clone();
            let mut n = 2;
            while by_name.contains_key(&name) {
                name = format!("{stem}-{n}");
                n += 1;
            }
            let served = Arc::new(Served {
                name: name.clone(),
                path,
                router: OnceCell::new(),
                recent: Mutex::new(None),
            });
            by_name.insert(name, Arc::clone(&served));
            order.push(served);
        }
        Ok(Self {
            env,
            order,
            by_name,
            live: None,
        })
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Arc<Served>> {
        self.by_name.get(name)
    }

    /// Each repository opened once, keyed by its URL name, for permalinks and run requests.
    pub(crate) fn targets(&self) -> io::Result<Vec<crate::actions::Target>> {
        self.order
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

impl Served {
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

/// A URL-safe name: no separator, no dot prefix (so never `.` or `..`), nothing to escape.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Expands the CLI paths: a repository stays as is, a plain folder is searched `depth` levels
/// down for repositories (skipping hidden folders and those in `skip`), and a folder with none
/// is taken as a path inside one.
pub fn discover_repos(paths: &[PathBuf], depth: usize, skip: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.join(".git").exists() {
            out.push(p.clone());
            continue;
        }
        let before = out.len();
        walk(p, depth, skip, &mut out);
        if out.len() == before {
            out.push(p.clone());
        }
    }
    out
}

fn walk(dir: &Path, depth: usize, skip: &[String], out: &mut Vec<PathBuf>) {
    if depth == 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut dirs: Vec<PathBuf> = rd
        .filter_map(std::result::Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            !n.starts_with('.') && !skip.iter().any(|s| s.as_str() == n)
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        if d.join(".git").exists() {
            out.push(d);
        } else {
            walk(&d, depth - 1, skip, out);
        }
    }
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
        .with_state(table)
}

async fn slash(uri: Uri) -> Redirect {
    Redirect::permanent(&format!("{}/", uri.path()))
}

/// Hands the request to the repository's own router with the `/r/NAME` prefix removed.
async fn dispatch(State(table): State<Arc<Table>>, req: Request) -> Response {
    let raw = req.uri().path().to_owned();
    let Some(after) = raw.strip_prefix("/r/") else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (seg, rest) = after.split_once('/').unwrap_or((after, ""));
    // The raw segment must match a table name byte for byte: an encoded `%2e%2e` or `%2F`
    // never equals a valid name, so it falls through to 404.
    let Some(served) = table.get(seg).cloned() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let router = match served
        .router
        .get_or_try_init(|| async { repo_router(&served, table.live.clone(), &table.env) })
        .await
    {
        Ok(r) => r.clone(),
        Err(e) => {
            tracing::warn!("opening {}: {e}", served.name);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let pq = match req.uri().query() {
        Some(q) => format!("/{rest}?{q}"),
        None => format!("/{rest}"),
    };
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
) -> io::Result<Router> {
    let base = format!("/r/{}", served.name);
    crate::repo_router(&served.path, base, served.name.clone(), live, env)
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
        #[allow(clippy::cast_precision_loss)]
        let x = i as f64 * 90.0 / (SPARK_WEEKS - 1) as f64;
        let y = 19.0 - f64::from(*n) * 18.0 / f64::from(max);
        let _ = write!(pts, "{x:.1},{y:.1} ");
    }
    pts.trim_end().to_owned()
}

async fn index(State(table): State<Arc<Table>>) -> crate::error::Result<Html<String>> {
    let html = tokio::task::spawn_blocking(move || {
        let now = fmt::now();
        let rows = table
            .order
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
                    peak,
                    href,
                }
            })
            .collect();
        IndexPage { rows }.render()
    })
    .await??;
    Ok(Html(html))
}

async fn repos_json(State(table): State<Arc<Table>>) -> Response {
    let items = tokio::task::spawn_blocking(move || {
        let now = fmt::now();
        table
            .order
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
mod tests {
    use super::valid_name;

    #[test]
    fn names() {
        for ok in ["tendir", "gitterrace-2", "a.b", "x_y"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", ".", "..", ".hidden", "a/b", "a b", "a%2e"] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@e.x"])
            .args(args)
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn recent_is_memoised_by_head() {
        let dir = tempfile::Builder::new()
            .prefix("repo")
            .tempdir()
            .expect("tmp");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        git(p, &["commit", "-q", "--allow-empty", "-m", "one"]);
        let table = super::Table::new(
            vec![p.to_path_buf()],
            crate::app::Env::new(gitterrace_config::Config::default()),
        )
        .expect("table");
        let s = table.order[0].clone();
        let now = crate::fmt::now() + 60;
        let first = s.recent(now);
        assert!(
            std::sync::Arc::ptr_eq(&first, &s.recent(now)),
            "same HEAD reuses the read"
        );
        git(p, &["commit", "-q", "--allow-empty", "-m", "two"]);
        let after = s.recent(now);
        let total = |r: &super::Recent| r.weeks.iter().sum::<u32>();
        assert_eq!((total(&first), total(&after)), (1, 2));
        assert_eq!(after.last.as_ref().map(|c| c.subject.as_str()), Some("two"));
    }

    /// The line counts every commit in the window, not the newest few thousand: a busy
    /// repository must not show an empty tail.
    #[test]
    fn weekly_counts_the_whole_window() {
        let now = 100 * super::WEEK;
        // 5000 commits evenly over the 26 weeks, far more than the old 2000-commit read.
        let step = super::WEEK * super::SPARK_WEEKS as i64 / 5000;
        let times: Vec<i64> = (0..5000).map(|i| now - i * step).collect();
        let weeks = super::weekly(&times, now);
        assert_eq!(weeks.len(), super::SPARK_WEEKS);
        assert!(weeks.iter().all(|n| *n > 0), "{weeks:?}");
        assert_eq!(weeks.iter().sum::<u32>(), 5000);
    }
}
