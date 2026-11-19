//! The warm-up: everything built ahead of the first visitor, in one registry, run before the
//! repository turns ready.
//!
//! Each [`Warmer`] names the pages it serves cold, so a route walk can prove every insights
//! page has one. The refresh runs them in order once the index holds HEAD, with the state at
//! warming; a page asked for meanwhile reads the same source and waits on the same render
//! lock, so the two never build one page twice.

use std::sync::Arc;

use tower::ServiceExt;

use crate::app::App;

/// One piece of warm-up work and the routes whose first request it makes cheap.
pub(crate) struct Warmer {
    pub(crate) name: &'static str,
    /// Routes served from what this builds. Empty for work every page reads (the repo id).
    pub(crate) routes: &'static [&'static str],
    run: Run,
}

/// How a warmer runs: on a blocking thread over the app, or as rendered pages.
enum Run {
    Blocking(fn(&App, &str)),
    /// The unfiltered pages of [`crate::WARM_PAGES`] that are switched on.
    Pages,
    Blame,
}

/// Every warmer, in the order they run. The shared tables come first: every per-file page
/// after them slices the tables instead of building its own.
pub(crate) const WARMERS: &[Warmer] = &[
    Warmer {
        name: "repo_id",
        routes: &[],
        run: Run::Blocking(repo_id),
    },
    Warmer {
        name: "shared_tables",
        routes: &["/stats/hotspots", "/stats/ownership", "/stats/languages"],
        run: Run::Blocking(crate::insights::warm_shared),
    },
    Warmer {
        name: "pages",
        routes: &crate::WARM_PAGES,
        run: Run::Pages,
    },
    Warmer {
        name: "blame",
        routes: &["/blame/{rev}/{*path}"],
        run: Run::Blame,
    },
];

/// Runs every warmer for `head`, in order. A warmer that fails only means its first visitor
/// builds what it would have.
pub(crate) async fn run(app: &Arc<App>, head: &str) {
    let _warm = app.warming();
    for w in WARMERS {
        tracing::debug!("{}: warming {} for {:?}", app.name, w.name, w.routes);
        match w.run {
            Run::Blocking(f) => {
                let (a, h) = (Arc::clone(app), head.to_owned());
                if let Err(e) = tokio::task::spawn_blocking(move || f(&a, &h)).await {
                    tracing::debug!("{}: warmer {} stopped: {e}", app.name, w.name);
                }
            }
            Run::Pages => pages(app).await,
            Run::Blame => {
                if app
                    .cfg()
                    .feature_on(gitterrace_config::Feature::Blame, Some(&app.name))
                {
                    crate::routes::prewarm_blame(Arc::clone(app)).await;
                }
            }
        }
    }
}

/// Every page's chrome shows the repo id, which walks the whole history once per process.
fn repo_id(a: &App, _head: &str) {
    a.repo.id();
}

/// Renders the switched-on [`crate::WARM_PAGES`] through a router of the app's own pages, so
/// each lands in the memo. Built here, not kept on the app: the router holds the app.
async fn pages(app: &Arc<App>) {
    let router = crate::routes::router(Arc::clone(app));
    for path in crate::features::warm(&app.cfg(), Some(&app.name)) {
        // Internal requests skip the Host and cross-site guard, which wraps the outer router.
        if let Ok(req) = axum::http::Request::get(path).body(axum::body::Body::empty()) {
            drop(router.clone().oneshot(req).await);
        }
    }
}

#[cfg(test)]
#[path = "warm_tests.rs"]
mod tests;
