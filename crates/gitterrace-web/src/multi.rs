//! Several repositories on one server: a name table fixed at startup, one lazily built
//! router per repository under `/r/NAME`, and a repository index at `/`.

use std::fmt::Write as _;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use askama::Template;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, Uri},
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
};
use gitterrace_git::Repo;
use tokio::sync::OnceCell;
use tower::ServiceExt;

use crate::fmt;

/// Folders never searched for repositories.
const SKIP_DIRS: [&str; 3] = ["node_modules", "target", ".wt"];
/// How deep below a given folder repositories are looked for.
const WALK_DEPTH: usize = 2;
/// Weeks shown in the index sparkline.
const SPARK_WEEKS: usize = 26;
/// Newest commits read per repository for the sparkline.
const SPARK_COMMITS: usize = 2000;
const WEEK: i64 = 7 * 86_400;

/// One served repository: its URL name and where it lives.
pub(crate) struct Served {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    router: OnceCell<Router>,
}

/// The startup repository table. A URL name resolves only through this map, never by
/// joining it onto a filesystem path.
pub(crate) struct Table {
    order: Vec<Arc<Served>>,
    by_name: HashMap<String, Arc<Served>>,
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
}

#[derive(serde::Serialize)]
struct RepoItem {
    name: String,
    href: String,
    when: String,
}

impl Table {
    /// Names each repository after its folder, deduplicated with a numeric suffix.
    pub(crate) fn new(paths: Vec<PathBuf>) -> io::Result<Self> {
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
            });
            by_name.insert(name, Arc::clone(&served));
            order.push(served);
        }
        Ok(Self { order, by_name })
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Arc<Served>> {
        self.by_name.get(name)
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

/// Expands the CLI paths: a repository stays as is, a plain folder is searched for
/// repositories, and a folder with none is taken as a path inside one.
pub fn discover_repos(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.join(".git").exists() {
            out.push(p.clone());
            continue;
        }
        let before = out.len();
        walk(p, WALK_DEPTH, &mut out);
        if out.len() == before {
            out.push(p.clone());
        }
    }
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
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
            !n.starts_with('.') && !SKIP_DIRS.contains(&n.as_ref())
        })
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for d in dirs {
        if d.join(".git").exists() {
            out.push(d);
        } else {
            walk(&d, depth - 1, out);
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
        .get_or_try_init(|| async { repo_router(&served) })
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

fn repo_router(served: &Served) -> io::Result<Router> {
    let base = format!("/r/{}", served.name);
    crate::repo_router(&served.path, base)
}

fn last(path: &Path) -> Option<gitterrace_git::CommitSummary> {
    let repo = Repo::discover(path).ok()?;
    let head = repo.resolve("HEAD").ok()?;
    repo.log(&head, None, 0, 1).ok()?.into_iter().next()
}

/// Commits per week over the last `SPARK_WEEKS` weeks as a polyline in a 90x20 box.
fn spark(path: &Path, now: i64) -> String {
    let mut weeks = [0u32; SPARK_WEEKS];
    if let Ok(repo) = Repo::discover(path)
        && let Ok(head) = repo.resolve("HEAD")
        && let Ok(list) = repo.log(&head, None, 0, SPARK_COMMITS)
    {
        for c in list {
            let age = (now - c.author.time).max(0) / WEEK;
            if let Ok(i) = usize::try_from(age)
                && i < SPARK_WEEKS
            {
                weeks[SPARK_WEEKS - 1 - i] += 1;
            }
        }
    }
    let max = weeks.iter().copied().max().unwrap_or(0).max(1);
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
                let c = last(&s.path);
                IndexRow {
                    name: s.name.clone(),
                    subject: c.as_ref().map(|c| c.subject.clone()).unwrap_or_default(),
                    when: c
                        .as_ref()
                        .map(|c| fmt::ago(c.author.time, now))
                        .unwrap_or_default(),
                    commit_href: c.as_ref().map_or_else(
                        || href.clone(),
                        |c| format!("/r/{}/commit/{}", s.name, c.id),
                    ),
                    spark: spark(&s.path, now),
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
                when: last(&s.path)
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
}
