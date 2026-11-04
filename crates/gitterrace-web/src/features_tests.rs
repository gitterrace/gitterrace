//! The `[features]` guards: every route belongs to a feature or to the core, a feature switched
//! off leaves no trace on any page, and no file can turn on what a public server forbids.

use std::collections::BTreeSet;
use std::path::Path;

use axum::{Router, body::Body, http::Request};
use gitterrace_config::{Config, Feature, INSIGHT_PAGES, keys, parse};
use gitterrace_testkit::{Fixture, base, get};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::features::{CORE_ROUTES, FEATURE_ROUTES, INSIGHT_ROUTES, route_on, warm};

/// Every route the server answers: the classified page routes, the API registry and the one
/// switchable asset.
fn every_route() -> BTreeSet<String> {
    crate::public::PUBLIC
        .iter()
        .chain(crate::public::LOCAL)
        .map(|r| (*r).to_owned())
        .chain(
            gitterrace_api::ENDPOINTS
                .iter()
                .map(|e| format!("/api/v1{}", e.path)),
        )
        .chain(std::iter::once(
            "/static/vendor/mermaid-11.17.2.min.js".to_owned(),
        ))
        .collect()
}

/// Class guard: a route added to any router, or to the API registry, fails here until it is
/// given to a feature in `FEATURE_ROUTES` or to `CORE_ROUTES`; one listed in neither would be
/// served with every switch off.
#[test]
fn every_route_is_a_feature_or_core() {
    let owned: BTreeSet<String> = FEATURE_ROUTES
        .iter()
        .flat_map(|(_, rs)| rs.iter().map(|r| (*r).to_owned()))
        .collect();
    let core: BTreeSet<String> = CORE_ROUTES.iter().map(|r| (*r).to_owned()).collect();
    assert!(owned.is_disjoint(&core), "{:?}", owned.intersection(&core));
    let classified: BTreeSet<String> = owned.union(&core).cloned().collect();
    assert_eq!(
        every_route(),
        classified,
        "put the route in FEATURE_ROUTES or CORE_ROUTES in features.rs"
    );
    for (page, route) in INSIGHT_ROUTES {
        assert!(INSIGHT_PAGES.contains(page), "{page}");
        assert!(owned.contains(*route), "{route}");
    }
    for page in INSIGHT_PAGES {
        assert!(
            INSIGHT_ROUTES.iter().any(|(p, _)| *p == page),
            "insights page {page} has no route"
        );
    }
}

/// What one switch turns off: a `[features]` key or one `insights.pages` entry.
#[derive(Debug, Clone)]
enum Off {
    Key(Feature),
    Page(&'static str),
}

impl Off {
    /// Every switch, generated from the settings table and the insights page list.
    fn all() -> Vec<Self> {
        let ks = keys()
            .into_iter()
            .filter(|k| k.section == "features")
            .map(|k| Self::Key(Feature::from_key(k.name).expect("a feature")));
        ks.chain(INSIGHT_PAGES.into_iter().map(Self::Page))
            .collect()
    }

    fn config(&self) -> Config {
        let text = match self {
            Self::Key(f) => format!("[features]\n{} = false\n", f.key()),
            Self::Page(p) => {
                let rest: Vec<String> = INSIGHT_PAGES
                    .iter()
                    .filter(|q| *q != p)
                    .map(|q| format!("\"{q}\""))
                    .collect();
                format!("[insights]\npages = [{}]\n", rest.join(", "))
            }
        };
        parse(&text).expect("parse").config
    }

    /// The routes this switch takes away.
    fn routes(&self) -> Vec<&'static str> {
        match self {
            Self::Key(Feature::Api) => gitterrace_api::ENDPOINTS
                .iter()
                .map(|e| e.path)
                .map(|p| &*Box::leak(format!("/api/v1{p}").into_boxed_str()))
                .collect(),
            Self::Key(f) => FEATURE_ROUTES
                .iter()
                .filter(|(g, _)| g == f)
                .flat_map(|(_, rs)| rs.iter().copied())
                .collect(),
            Self::Page(p) => INSIGHT_ROUTES
                .iter()
                .filter(|(q, _)| q == p)
                .map(|(_, r)| *r)
                .collect(),
        }
    }
}

/// A URL for a route pattern on the fixture, relative to the repository when `in_repo`.
fn sample(route: &str, head: &str) -> (bool, String) {
    let url = route
        .replace("{rev}", "HEAD")
        .replace("{*path}", "src/a.rs")
        .replace("{id}", head)
        .replace("{index}", "0")
        .replace("{name}", "activity");
    let server_wide = route.starts_with("/_")
        || route.starts_with("/r/")
        || route.starts_with("/static/")
        || route == "/api/repos";
    let url = if route.starts_with("/r/{name}") {
        route.replace("{name}", "repo")
    } else {
        url
    };
    (!server_wide, url)
}

/// Whether the link `link` (repository prefix removed) points at `route`.
fn links_to(link: &str, route: &str) -> bool {
    let path = link.split(['?', '#']).next().unwrap_or(link);
    match route.split_once('{') {
        None => path == route,
        Some((lit, _)) => path.starts_with(lit) && path.len() > lit.len(),
    }
}

/// Every `href`, `src` and `action` value of a page.
fn links(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attr in ["href=\"", "src=\"", "action=\""] {
        for rest in html.split(attr).skip(1) {
            out.push(rest.split('"').next().unwrap_or("").replace("&amp;", "&"));
        }
    }
    out
}

fn fixture(dir: &Path) -> std::path::PathBuf {
    Fixture::init(&dir.join("repo"))
        .write(
            "README.md",
            "---\ntitle: front\n---\n# Hi\n\n[a](src/a.rs) ![i](img.png)\n\n```mermaid\ngraph TD; A-->B\n```\n\n## One\n\n## Two\n\n## Three\n",
        )
        .write("src/a.rs", "fn main() {}\n")
        .write("img.png", "\u{89}PNG\r\n")
        .commit("feat(core): first")
        .write("src/a.rs", "fn main() {\n}\n")
        .commit("fix: second")
        .path()
        .to_owned()
}

async fn ready(r: &Router, prefix: &str) {
    for _ in 0..600 {
        let (_, js) = get(r, &format!("{prefix}/api/progress")).await;
        if js.contains("\"ready\":true") {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("never ready");
}

/// The core pages every crawl renders: each could link any feature.
const CORE_PAGES: [&str; 8] = [
    "/",
    "/log/HEAD",
    "/tree/HEAD",
    "/blob/HEAD/README.md",
    "/blob/HEAD/src/a.rs",
    "/blob/HEAD/img.png",
    "/refs",
    "/api/v1",
];

/// Class guard: for every switch the settings table and `insights.pages` have, turned off on
/// its own, each route it owns answers what an unknown path answers, no core page (nor the
/// API index) links it, and the warm-up skips it. A new key with a route but no gate, or a
/// page that links a route without asking the table, fails here.
#[tokio::test]
async fn features_off_leave_no_trace() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture(dir.path());
    let head = gitterrace_testkit::git(&p, &["rev-parse", "HEAD"]);
    for off in Off::all() {
        let cfg = off.config();
        let r = crate::app_router_with(&p, cfg.clone()).expect("router");
        let prefix = base(&r).await;
        ready(&r, &prefix).await;
        let (_, unknown) = get(&r, &format!("{prefix}/no/such/page")).await;
        let (_, unknown_outer) = get(&r, "/no/such/page").await;
        let routes = off.routes();
        for route in &routes {
            let (in_repo, url) = sample(route, &head);
            let url = if in_repo {
                format!("{prefix}{url}")
            } else {
                url
            };
            let (s, body) = get(&r, &url).await;
            assert_eq!(s, 404, "{off:?}: {url}");
            assert!(
                body == unknown || body == unknown_outer,
                "{off:?}: {url} answers unlike an unknown page"
            );
            assert!(
                !warm(&cfg, Some("repo")).contains(route),
                "{off:?} warms {route}"
            );
        }
        let mut pages: Vec<String> = CORE_PAGES.iter().map(|p| (*p).to_owned()).collect();
        pages.push(format!("/commit/{head}"));
        for page in pages {
            if matches!(off, Off::Key(Feature::Api)) && page == "/api/v1" {
                continue;
            }
            let (s, html) = get(&r, &format!("{prefix}{page}")).await;
            assert_eq!(s, 200, "{off:?}: core page {page}");
            for link in links(&html) {
                let local = link.strip_prefix(&prefix).unwrap_or(&link);
                for route in &routes {
                    assert!(!links_to(local, route), "{off:?}: {page} links {link}");
                }
            }
            for route in &routes {
                let api = route.strip_prefix("/api/v1").filter(|_| page == "/api/v1");
                if let Some(a) = api.filter(|a| *a != "/") {
                    assert!(
                        !html.contains(&format!("\"path\":\"{a}\"")),
                        "{off:?}: the API index lists {a}"
                    );
                }
            }
            check_inline(&off, &page, &html);
        }
    }
}

/// The switches without a route of their own show in the page they change.
fn check_inline(off: &Off, page: &str, html: &str) {
    // The README shows on the overview card and on its own page.
    if matches!(off, Off::Key(Feature::MarkdownPreview)) && page == "/" {
        for trace in ["markdown-body", "data-outline"] {
            assert!(
                !html.contains(trace),
                "preview off: the overview shows {trace}"
            );
        }
        assert!(
            html.contains("<pre>"),
            "preview off: no plain README on the overview"
        );
    }
    if page != "/blob/HEAD/README.md" {
        return;
    }
    match off {
        Off::Key(Feature::FrontMatter) => {
            assert!(!html.contains("class=\"fm\""), "front matter rendered");
        }
        Off::Key(Feature::Mermaid) => {
            assert!(!html.contains("class=\"mermaid\""), "mermaid block kept");
        }
        Off::Key(Feature::MarkdownPreview) => {
            for trace in [
                "markdown-body",
                "data-show=",
                "data-outline",
                "data-md-view",
            ] {
                assert!(!html.contains(trace), "preview off still shows {trace}");
            }
        }
        _ => {}
    }
}

/// The default config renders the page chrome as before: no `gt-off`, the search box, every
/// insights tab in the old order.
#[tokio::test]
async fn defaults_add_nothing_to_a_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture(dir.path());
    let r = crate::app_router_with(&p, Config::default()).expect("router");
    let prefix = base(&r).await;
    let (_, html) = get(&r, &format!("{prefix}/")).await;
    assert!(!html.contains("gt-off"));
    assert!(html.contains("class=\"topsearch\""));
    assert!(html.contains("id=\"reposwitch\""));
    let all = crate::features::Effective::of(&Config::default(), None);
    let tabs = crate::features::with(all, || crate::views::stats_tabs("a"));
    let labels: Vec<&str> = tabs
        .primary
        .iter()
        .chain(&tabs.more)
        .map(|n| n.label)
        .collect();
    assert_eq!(labels, crate::views::stats_tab_labels());
    assert_eq!(warm(&Config::default(), None), crate::WARM_PAGES);
}

/// `insights.pages` orders the tabs and the Insights link opens the first one listed.
#[test]
fn insights_pages_order_the_tabs() {
    let cfg = parse("[insights]\npages = [\"types\", \"activity\"]\n")
        .expect("parse")
        .config;
    let e = crate::features::Effective::of(&cfg, None);
    let (labels, home) = crate::features::with(e, || {
        let t = crate::views::stats_tabs("t");
        let l: Vec<&str> = t.primary.iter().map(|n| n.label).collect();
        (l, crate::features::insights_home())
    });
    assert_eq!(labels, ["Commit types", "Activity", "Compare"]);
    assert_eq!(home, Some("/stats/types"));
}

/// A public server turns off the local-only routes and, unless the file asks, the API,
/// whatever `[features]` says.
#[test]
fn the_file_never_reopens_a_public_route() {
    let on = Feature::ALL
        .map(|f| [f.key(), " = true\n"].concat())
        .concat();
    let cfg = parse(&format!("[server]\nis_public = true\n[features]\n{on}"))
        .expect("parse")
        .config;
    for r in crate::public::LOCAL {
        let owned = FEATURE_ROUTES.iter().any(|(_, rs)| rs.contains(r));
        assert!(
            owned && !route_on(&cfg, None, r),
            "{r} is reachable when public"
        );
    }
    let plain = parse("[server]\nis_public = true\n").expect("parse").config;
    assert!(!route_on(&plain, None, "/api/v1/repo"));
    assert!(route_on(&cfg, None, "/api/v1/repo"));
}

/// The router of a public server answers 404 on the API by default and on the working tree
/// even with every switch set.
#[tokio::test]
async fn a_public_server_keeps_its_gates() {
    const HOST: &str = "git.example.com";
    let dir = tempfile::tempdir().expect("tmp");
    let p = fixture(dir.path());
    let on = Feature::ALL
        .into_iter()
        .filter(|f| *f != Feature::Api)
        .map(|f| [f.key(), " = true\n"].concat())
        .collect::<String>();
    let text = format!("[server]\nis_public = true\nhosts = [\"{HOST}\"]\n[features]\n{on}");
    let r = crate::app_router_with(&p, parse(&text).expect("parse").config).expect("router");
    let send = |uri: &str| {
        let req = Request::get(uri)
            .header("host", HOST)
            .body(Body::empty())
            .expect("req");
        let r = r.clone();
        async move {
            let res = r.oneshot(req).await.expect("response");
            let s = res.status().as_u16();
            res.into_body().collect().await.expect("body");
            s
        }
    };
    assert_eq!(send("/r/repo/").await, 200);
    for uri in [
        "/r/repo/worktree",
        "/r/repo/events",
        "/r/repo/api/v1/repo",
        "/_actions/status",
    ] {
        assert_eq!(send(uri).await, 404, "{uri}");
    }
}
