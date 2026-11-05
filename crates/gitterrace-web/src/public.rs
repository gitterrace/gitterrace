//! What a public server (`server.is_public`) answers: committed history and nothing of this
//! machine. Every route literal is classified below; a public server answers only the
//! `PUBLIC` ones and 404s the rest, so a route added without a class stays dark there.

use axum::{
    extract::{MatchedPath, Request},
    middleware::Next,
    response::Response,
};

/// Routes that show committed history, refs or static assets only.
pub(crate) const PUBLIC: &[&str] = &[
    "/",
    "/log",
    "/log/{rev}",
    "/commit/{id}",
    "/diff/{rev}/{index}",
    "/tree/{rev}",
    "/tree/{rev}/",
    "/tree/{rev}/{*path}",
    "/blob/{rev}/{*path}",
    "/blame/{rev}/{*path}",
    "/hl/{rev}/{*path}",
    "/raw/{rev}/{*path}",
    "/refs",
    "/tag/{name}",
    "/search",
    "/compare",
    "/history/{*path}",
    "/stats",
    "/stats/types",
    "/stats/languages",
    "/stats/frequency",
    "/stats/hotspots",
    "/stats/ownership",
    "/stats/patterns",
    "/stats/contributors",
    "/api/files/{rev}",
    "/api/refs",
    "/api/last/{rev}",
    "/api/last/{rev}/{*path}",
    "/api/progress",
    "/api/repos",
    "/r/{name}",
    "/r/{name}/",
    "/r/{name}/{*rest}",
    "/go/{*rest}",
];

/// Routes that show this machine (working tree, stashes, linked worktree paths, live
/// change streams, actions and their targets) or change the server. Never public.
#[cfg(test)]
pub(crate) const LOCAL: &[&str] = &[
    "/worktree",
    "/worktree/diff/{*path}",
    "/worktrees",
    "/stashes",
    "/events",
    "/_actions/status",
    "/_actions/run",
    "/_send/targets",
    "/r/{name}/_send/targets",
    "/_repos",
    "/_settings",
    "/_settings/file",
    "/_settings/check",
];

/// Whether a matched route may answer on a public server. `/api/v1` is the read-only JSON
/// registry, every endpoint of which reads committed objects through the restricted handle.
fn public_route(route: &str) -> bool {
    PUBLIC.contains(&route)
        || route.starts_with("/static/")
        || route
            .strip_prefix("/api/v1")
            .is_some_and(|r| gitterrace_api::ENDPOINTS.iter().any(|e| e.path == r))
}

/// Route layer of a public server: an unclassified or local route answers 404, as if absent.
pub(crate) async fn only_public(req: Request, next: Next) -> Response {
    let allowed = req
        .extensions()
        .get::<MatchedPath>()
        .is_some_and(|m| public_route(m.as_str()));
    if allowed {
        next.run(req).await
    } else {
        crate::error::not_found("Not served here.")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{LOCAL, PUBLIC, public_route};

    /// Every `.route("<literal>"` and route constant in the router sources, `/static` aside.
    fn routed() -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for src in [
            include_str!("routes.rs"),
            include_str!("multi.rs"),
            include_str!("actions.rs"),
            include_str!("add.rs"),
            include_str!("settings.rs"),
        ] {
            for rest in src.split(".route(").skip(1) {
                let rest = rest.trim_start();
                let lit = rest.strip_prefix('"').map_or_else(
                    // A constant: resolve the ones the routers use.
                    || match rest.split([',', ')']).next() {
                        Some("STATUS_PATH") => Some("/_actions/status".to_owned()),
                        Some("RUN_PATH") => Some(crate::actions::RUN_PATH.to_owned()),
                        Some(other) if other.starts_with("&format!") => None,
                        Some(other) => panic!("unclassifiable route expression {other}"),
                        None => None,
                    },
                    |r| r.split('"').next().map(str::to_owned),
                );
                if let Some(l) = lit.filter(|l| !l.starts_with("/static")) {
                    out.insert(l);
                }
            }
        }
        out
    }

    /// A route added to any router fails here until it is put in `PUBLIC` or `LOCAL`.
    #[test]
    fn every_route_is_classified_once() {
        let public: BTreeSet<String> = PUBLIC.iter().map(|s| (*s).to_owned()).collect();
        let local: BTreeSet<String> = LOCAL.iter().map(|s| (*s).to_owned()).collect();
        assert!(
            public.is_disjoint(&local),
            "{:?}",
            public.intersection(&local)
        );
        let classified: BTreeSet<String> = public.union(&local).cloned().collect();
        assert_eq!(
            routed(),
            classified,
            "classify the route in public.rs: PUBLIC shows committed history only"
        );
    }

    #[test]
    fn local_routes_are_never_public() {
        for r in LOCAL {
            assert!(!public_route(r), "{r}");
        }
        assert!(public_route("/api/v1/repo"));
        assert!(!public_route("/api/v1/nope"));
        assert!(public_route("/static/app.css"));
    }
}
