//! Which routes each `[features]` switch owns, and the one gate that answers a route of an off
//! feature with the 404 of an unknown page. Pages read the same table through [`on`], so an off
//! feature leaves no link, menu entry or script behind. Off always wins: the gate only ever
//! narrows what `public.rs` and the loopback checks already allow.

use std::{cell::RefCell, sync::Arc};

use axum::{
    Json,
    extract::{MatchedPath, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use gitterrace_config::{Config, Feature, INSIGHT_PAGES};

use crate::app::{App, Env};

/// The routes each feature owns. A route listed under two features needs both.
/// Every `/api/v1` route is also the `api_enabled` feature's, see [`features_of`].
pub(crate) const FEATURE_ROUTES: &[(Feature, &[&str])] = &[
    (Feature::Blame, &["/blame/{rev}/{*path}", "/api/v1/blame"]),
    (Feature::Search, &["/search"]),
    (Feature::Raw, &["/raw/{rev}/{*path}"]),
    (Feature::History, &["/history/{*path}"]),
    (Feature::Compare, &["/compare", "/api/v1/compare"]),
    (
        Feature::Worktree,
        &[
            "/worktree",
            "/_checkpoint",
            "/_viewed",
            "/_viewed/all",
            "/_findings",
            "/api/v1/worktree",
            "/api/v1/checkpoints",
            "/api/v1/viewed",
            "/api/v1/findings",
            "/worktree/diff/{*path}",
            "/worktree/find",
            "/worktree/raw/{*path}",
            "/worktree/blob/{*path}",
            "/worktrees",
            "/stashes",
            "/_dirty",
        ],
    ),
    (
        Feature::Insights,
        &[
            "/stats",
            "/stats/languages",
            "/stats/hotspots",
            "/stats/ownership",
            "/stats/ownership/{author}",
            "/stats/contributors",
            "/api/v1/contributors",
            "/api/v1/file_insight",
            "/api/v1/review_plan",
            "/api/insight/{rev}",
            "/api/insight/{rev}/{*path}",
            "/api/v1/insights",
            "/api/v1/insights/{name}",
        ],
    ),
    (Feature::Live, &["/events"]),
    (
        Feature::Actions,
        &["/_actions/status", "/_actions/run", "/_send/targets", "/r/{name}/_send/targets"],
    ),
    (Feature::RepoAdmin, &["/_repos", "/_launch"]),
    (Feature::Settings, &["/_settings", "/_settings/file", "/_settings/check"]),
    (Feature::Picker, &["/api/repos"]),
    (Feature::Mermaid, &[crate::MERMAID_JS]),
];

/// The route of each `insights.pages` entry that is a page; a page off in the list answers 404
/// there. `types`, `frequency` and `patterns` are sections of another page and own no route.
pub(crate) const INSIGHT_ROUTES: &[(&str, &str)] = &[
    ("activity", "/stats"),
    ("contributors", "/stats/contributors"),
    ("contributors", "/api/v1/contributors"),
    ("hotspots", "/stats/hotspots"),
    ("languages", "/stats/languages"),
    ("ownership", "/stats/ownership"),
    ("ownership", "/stats/ownership/{author}"),
];

const API: &str = "/api/v1";

/// Features JS reads from the `gt-off` meta: they own a key, a menu entry or a fetch.
const SCRIPTED: [Feature; 5] =
    [Feature::Search, Feature::Insights, Feature::Raw, Feature::Actions, Feature::Picker];

/// The features a route needs, every one of them on.
pub(crate) fn features_of(route: &str) -> impl Iterator<Item = Feature> + '_ {
    FEATURE_ROUTES
        .iter()
        .filter(move |(_, rs)| rs.contains(&route))
        .map(|(f, _)| *f)
        .chain(route.starts_with(API).then_some(Feature::Api))
}

/// Whether `route` (a matched route pattern) is served for `repo` under `cfg`.
pub(crate) fn route_on(cfg: &Config, repo: Option<&str>, route: &str) -> bool {
    features_of(route).all(|f| cfg.feature_on(f, repo))
        && INSIGHT_ROUTES
            .iter()
            .filter(|(_, r)| *r == route)
            .all(|(p, _)| cfg.insight_page_on(p, repo))
}

/// The answer of a route whose feature is off: the page an unknown path gets, byte for byte.
async fn absent() -> Response {
    crate::error::fallback().await
}

/// The `/api/v1/` index route, as the router matches it.
fn is_api_index(route: &str) -> bool {
    route == API || route == "/api/v1/"
}

pub(crate) async fn repo_gate(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let Some(route) = req.extensions().get::<MatchedPath>().map(|m| m.as_str().to_owned()) else {
        return next.run(req).await;
    };
    let cfg = app.cfg();
    let repo = Some(app.name.as_str());
    if !route_on(&cfg, repo, &route) {
        return absent().await;
    }
    if is_api_index(&route) {
        let keep = |p: &str| route_on(&cfg, repo, &format!("{API}{p}"));
        let insights = cfg.feature_on(Feature::Insights, repo);
        // Untouched when nothing is off, so the default index is the registry's own answer.
        if !insights || gitterrace_api::ENDPOINTS.iter().any(|e| !keep(e.path)) {
            return Json(gitterrace_api::index_body(keep, insights)).into_response();
        }
    }
    next.run(req).await
}

pub(crate) async fn server_gate(State(env): State<Env>, req: Request, next: Next) -> Response {
    let on = req
        .extensions()
        .get::<MatchedPath>()
        .is_none_or(|m| route_on(&env.cfg(), None, m.as_str()));
    if on { next.run(req).await } else { absent().await }
}

/// What one page may show: the features off for its repository and the insights pages in
/// tab order.
#[derive(Debug, Clone)]
pub(crate) struct Effective {
    off: Vec<Feature>,
    pages: Vec<&'static str>,
    /// Off by the file rather than by the public policy, for the `gt-off` meta.
    chosen_off: Vec<Feature>,
}

impl Effective {
    pub(crate) fn of(cfg: &Config, repo: Option<&str>) -> Self {
        let off: Vec<Feature> =
            Feature::ALL.into_iter().filter(|f| !cfg.feature_on(*f, repo)).collect();
        let chosen_off =
            off.iter().copied().filter(|f| !(cfg.server.is_public && f.is_local_only())).collect();
        let pages = cfg
            .insight_pages(repo)
            .into_iter()
            .filter_map(|p| INSIGHT_PAGES.iter().copied().find(|q| *q == p))
            .collect();
        Self { off, pages, chosen_off }
    }
}

thread_local! {
    /// The features of the page rendered on this thread; `None` means every one is on.
    static NOW: RefCell<Option<Arc<Effective>>> = const { RefCell::new(None) };
}

/// Runs `f` with `e` in force for [`on`], restoring the previous value afterwards, also on
/// a panic, so one request's switches never stay on a pooled thread.
pub(crate) fn with<T>(e: Effective, f: impl FnOnce() -> T) -> T {
    crate::fmt::scoped(&NOW, Some(Arc::new(e)), f)
}

fn now<T>(f: impl FnOnce(Option<&Effective>) -> T) -> T {
    NOW.with(|n| f(n.borrow().as_deref()))
}

pub(crate) fn on(f: Feature) -> bool {
    now(|e| e.is_none_or(|e| !e.off.contains(&f)))
}

/// `fmt::href(path)` when `f` is on for the page being rendered, else empty: a template shows
/// the link only when it is not empty.
pub(crate) fn href_if(f: Feature, path: &str) -> String {
    if on(f) { crate::fmt::href(path) } else { String::new() }
}

pub(crate) fn pages() -> Vec<&'static str> {
    now(|e| e.map_or_else(|| INSIGHT_PAGES.to_vec(), |e| e.pages.clone()))
}

/// `html()` when the insights page `name` is served, else empty: a page section left out of
/// `insights.pages` is not built.
pub(crate) fn section(name: &str, html: impl FnOnce() -> String) -> String {
    if pages().contains(&name) { html() } else { String::new() }
}

pub(crate) fn page_route(page: &str) -> Option<&'static str> {
    INSIGHT_ROUTES.iter().find(|(p, r)| *p == page && !r.starts_with(API)).map(|(_, r)| *r)
}

/// The link of the insights page named `page`, empty when it is not served.
pub(crate) fn page_href(page: &str) -> String {
    if pages().contains(&page) {
        page_route(page).map(crate::fmt::href).unwrap_or_default()
    } else {
        String::new()
    }
}

/// Where Insights starts: the first page served, `None` when none is.
pub(crate) fn insights_home() -> Option<&'static str> {
    // A section listed first owns no route: the first entry that is a page is home.
    pages().iter().find_map(|p| page_route(p))
}

/// The `gt-off` meta content: the scripted features the file turned off, empty when none.
pub(crate) fn scripted_off() -> String {
    now(|e| {
        e.map_or_else(String::new, |e| {
            SCRIPTED
                .iter()
                .filter(|f| e.chosen_off.contains(f))
                .map(|f| f.key().trim_end_matches("_enabled"))
                .collect::<Vec<_>>()
                .join(" ")
        })
    })
}

/// The unfiltered pages to render ahead of visitors: [`crate::WARM_PAGES`] without those off.
pub(crate) fn warm(cfg: &Config, repo: Option<&str>) -> Vec<&'static str> {
    crate::WARM_PAGES.into_iter().filter(|p| route_on(cfg, repo, p)).collect()
}

#[cfg(test)]
#[path = "features_tests.rs"]
mod tests;
