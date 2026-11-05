//! Read-only JSON API (axum router) and MCP server over gitterrace-git and gitterrace-index.
//!
//! Both surfaces call the same [`ApiState`] methods, so they return the same data.

#![expect(
    clippy::unused_async_trait_impl,
    reason = "the trait fixes the async signature"
)]

mod core;
mod endpoints;
mod error;
mod http;
mod insights;
mod mcp;
mod wire;

pub use core::{ApiState, options_from};
pub use endpoints::{ENDPOINTS, Endpoint, Param, generator as schema_generator};
pub use error::{ApiError, ERROR_CODES, SERVER_ERROR_MESSAGE};
pub use http::{index_body, router};
pub use insights::{
    HEAD_INSIGHTS, INSIGHTS, Insight, OWNERSHIP_DEPTH, Params, list as insight_list,
};
pub use mcp::{McpServer, ToolDoc, serve_mcp};

#[cfg(test)]
mod limit_tests;
#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::{ApiState, McpServer, router};

    fn fixture() -> (tempfile::TempDir, ApiState) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "one\ntwo\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "feat(core): first"]);
        std::fs::create_dir(p.join("src")).expect("mkdir");
        std::fs::write(p.join("src/b.rs"), "fn main() {}\n").expect("write");
        std::fs::write(p.join("a.txt"), "one\ntwo\nthree\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "fix: second"]);
        let state = ApiState::open(p).expect("open");
        state.refresh_index().expect("index");
        (dir, state)
    }

    async fn get(state: &ApiState, uri: &str) -> (StatusCode, Value) {
        let res = router(state.clone())
            .oneshot(Request::get(uri).body(Body::empty()).expect("req"))
            .await
            .expect("res");
        let status = res.status();
        let bytes = res.into_body().collect().await.expect("body").to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn read_endpoints() {
        let (_d, s) = fixture();
        let (st, v) = get(&s, "/repo").await;
        assert_eq!(st, StatusCode::OK);
        assert_eq!(v["totals"]["commits"], 2);
        let (_, v) = get(&s, "/log?per_page=1").await;
        assert_eq!(v["commits"].as_array().map(Vec::len), Some(1));
        assert_eq!(v["has_more"], true);
        let (_, v) = get(&s, "/log?path=src").await;
        assert_eq!(v["commits"][0]["subject"], "fix: second");
        let (_, v) = get(&s, "/commits/HEAD").await;
        assert_eq!(v["files"].as_array().map(Vec::len), Some(2));
        assert!(v["files"][0]["hunks"].is_array());
        let (_, v) = get(&s, "/tree").await;
        assert_eq!(v["entries"].as_array().map(Vec::len), Some(2));
        let (_, v) = get(&s, "/blob?path=a.txt").await;
        assert_eq!(v["text"], "one\ntwo\nthree");
        let (_, v) = get(&s, "/blame?path=a.txt").await;
        assert_eq!(v["lines"].as_array().map(Vec::len), Some(3));
        let (_, v) = get(&s, "/refs").await;
        assert_eq!(v["refs"][0]["name"], "main");
        let (_, v) = get(&s, "/contributors").await;
        assert_eq!(v["bus_factor"], 1);
    }

    #[tokio::test]
    async fn results_carry_go_permalinks() {
        let (_d, s) = fixture();
        let id = s.repo.id().expect("id");
        let (_, head) = get(&s, "/commits/HEAD").await;
        let sha = head["id"].as_str().expect("sha").to_owned();
        let base = format!("/go/{id}/{sha}");
        assert_eq!(head["permalink"], base.as_str());
        let file = head["files"]
            .as_array()
            .and_then(|f| f.iter().find(|f| f["path"] == "src/b.rs"))
            .expect("file");
        assert_eq!(file["permalink"], format!("{base}/src/b.rs").as_str());

        let (_, log) = get(&s, "/log").await;
        assert_eq!(log["commits"][0]["permalink"], base.as_str());

        let (_, blob) = get(&s, "/blob?path=a.txt").await;
        assert_eq!(blob["permalink"], format!("{base}/a.txt#L1-L3").as_str());
        let (_, blame) = get(&s, "/blame?path=a.txt").await;
        assert_eq!(blame["permalink"], format!("{base}/a.txt").as_str());

        let (_, hot) = get(&s, "/insights/hotspots").await;
        let row = hot["data"]["files"]
            .as_array()
            .and_then(|r| r.iter().find(|r| r["path"] == "a.txt"))
            .expect("row");
        assert_eq!(row["permalink"], format!("{base}/a.txt").as_str());
        let gt = format!("gitterrace://{id}/{sha}");
        assert_eq!(head["link"], gt.as_str(), "link defaults to gitterrace://");
        assert_eq!(blob["link"], format!("{gt}/a.txt#L1-L3").as_str());
        assert_eq!(row["link"], format!("{gt}/a.txt").as_str());

        let go = s.clone().with_link_style(gitterrace_types::LinkStyle::Go);
        let (_, head) = get(&go, "/commits/HEAD").await;
        assert_eq!(head["link"], base.as_str(), "links.style = go");
        assert_eq!(head["permalink"], base.as_str(), "permalink stays /go");
    }

    #[tokio::test]
    async fn every_insight_answers() {
        let (_d, s) = fixture();
        for (name, _) in crate::insight_list() {
            let (st, v) = get(&s, &format!("/insights/{name}")).await;
            assert_eq!(st, StatusCode::OK, "{name}");
            assert!(v.get("data").is_some());
        }
        let (st, _) = get(&s, "/insights/nope").await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn errors_and_etag() {
        let (_d, s) = fixture();
        let (st, v) = get(&s, "/blob?path=missing").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert_eq!(v["error"]["code"], "path_not_found");
        let (st, _) = get(&s, "/blob").await;
        assert_eq!(st, StatusCode::BAD_REQUEST);
        let res = router(s.clone())
            .oneshot(Request::get("/repo").body(Body::empty()).expect("req"))
            .await
            .expect("res");
        let tag = res.headers().get(header::ETAG).expect("etag").clone();
        let res = router(s)
            .oneshot(
                Request::get("/repo")
                    .header(header::IF_NONE_MATCH, tag)
                    .body(Body::empty())
                    .expect("req"),
            )
            .await
            .expect("res");
        assert_eq!(res.status(), StatusCode::NOT_MODIFIED);
    }

    #[test]
    fn one_huge_line_is_capped_by_bytes() {
        let (d, _) = fixture();
        let p = d.path();
        std::fs::write(p.join("min.js"), "é".repeat(400 * 1024)).expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "chore: minified"]);
        let s = ApiState::open(p).expect("open");
        let v =
            serde_json::to_value(s.blob(None, "min.js", None, None).expect("blob")).expect("json");
        assert!(v["text"].as_str().expect("text").len() <= 256 * 1024);
        assert_eq!(v["truncated"], true);
    }

    #[test]
    fn etag_moves_with_any_ref_not_only_head() {
        let (d, s) = fixture();
        git(d.path(), &["branch", "side", "HEAD~1"]);
        let before = s.etag().expect("tag");
        git(d.path(), &["branch", "-f", "side", "HEAD"]);
        assert_ne!(
            before,
            s.etag().expect("tag"),
            "side branch moved, HEAD did not"
        );
    }

    #[test]
    fn records_cache_is_shared_and_follows_the_tip() {
        let (dir, state) = fixture();
        let first = state.records("HEAD").expect("records");
        let second = state.records("HEAD").expect("records");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "second call reparsed"
        );
        assert_eq!(first.len(), 2);
        std::fs::write(dir.path().join("c.txt"), "x\n").expect("write");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "feat: third"]);
        state.refresh_index().expect("index");
        let moved = state.records("HEAD").expect("records");
        assert_eq!(moved.len(), 3, "moved tip must change the answer");
        let earlier = state.records("HEAD~1").expect("records");
        assert_eq!(
            earlier.len(),
            2,
            "another rev must not reuse the HEAD entry"
        );
    }

    /// A route and a registry entry are one thing: every registered path answers (not 404/405).
    #[tokio::test]
    async fn every_registered_endpoint_is_routed() {
        let (_d, s) = fixture();
        for e in crate::ENDPOINTS {
            let uri = e.path.replace("{id}", "HEAD").replace("{name}", "totals");
            // A handler may refuse the probe path with a JSON error. An unrouted path answers
            // with an empty body.
            let query = if e.path == "/tree" { "src" } else { "a.txt" };
            let (st, v) = get(&s, &format!("{uri}?path={query}")).await;
            assert!(st == StatusCode::OK || !v.is_null(), "{} unrouted", e.path);
        }
        let (_, v) = get(&s, "/").await;
        assert_eq!(
            v["endpoints"].as_array().map(Vec::len),
            Some(crate::ENDPOINTS.len())
        );
    }

    #[test]
    fn mcp_lists_read_only_tools() {
        let docs = McpServer::tool_docs();
        assert!(
            docs.iter().all(|t| t.read_only),
            "every tool must be read only"
        );
        let names: Vec<String> = docs.into_iter().map(|t| t.name).collect();
        for want in [
            "activity",
            "blame",
            "contributors",
            "hotspots",
            "list_tree",
            "log",
            "read_file",
            "repo_overview",
            "show_commit",
        ] {
            assert!(names.iter().any(|n| n == want), "missing {want}");
        }
    }

    /// CI checkouts are shallow: missing parents must degrade records, not fail the index.
    #[test]
    fn shallow_clone_indexes_and_answers() {
        let (src, _) = fixture();
        let dir = tempfile::tempdir().expect("tempdir");
        let url = format!("file://{}", src.path().display());
        git(dir.path(), &["clone", "-q", "--depth", "1", &url, "c"]);
        let state = ApiState::open(&dir.path().join("c")).expect("open");
        state.refresh_index().expect("index");
        assert_eq!(state.records("HEAD").expect("records").len(), 1);
        assert_eq!(state.log(None, None, 1, 10).expect("log").commits.len(), 1);
    }

    #[test]
    fn hostile_paging_and_rev_syntax_are_refused() {
        let (_d, s) = fixture();
        let huge = s.log(None, None, usize::MAX, 2);
        assert!(
            matches!(huge, Err(crate::ApiError::BadRequest(_))),
            "{huge:?}"
        );
        assert!(s.log(None, Some("a.txt"), 25_001, 2).is_ok());
        assert!(s.log(None, Some("a.txt"), 25_002, 2).is_err());
        for rev in [":/first", "HEAD^{/first}", "main@{1}"] {
            let r = s.log(Some(rev), None, 1, 1);
            assert!(matches!(r, Err(crate::ApiError::BadRequest(_))), "{rev}");
            assert!(s.commit(rev, false, None).is_err(), "{rev}");
        }
        assert!(s.log(Some("HEAD~1"), None, 1, 1).is_ok());
    }

    /// Every bounded answer stays bounded when the client asks for more, or asks nothing.
    #[test]
    fn caller_cannot_lift_caps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        let big = "x\n".repeat(gitterrace_config::Api::default().diff_lines + 10);
        std::fs::write(p.join("big.txt"), big).expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "big"]);
        let s = ApiState::open(p).expect("open");
        let lines = |v: &serde_json::Value| -> usize {
            v["files"][0]["hunks"].as_array().map_or(0, |hs| {
                hs.iter()
                    .map(|h| h["lines"].as_array().map_or(0, Vec::len))
                    .sum()
            })
        };
        for asked in [None, Some(usize::MAX)] {
            let v =
                serde_json::to_value(s.commit("HEAD", true, asked).expect("commit")).expect("json");
            assert!(
                lines(&v) <= gitterrace_config::Api::default().diff_lines,
                "{asked:?}"
            );
            assert_eq!(v["truncated"], true, "{asked:?}");
        }
        let v = s.log(None, None, 1, usize::MAX).expect("log");
        assert!(v.commits.len() <= gitterrace_config::Api::default().log_page_commits);
    }

    /// Tools that return free text from the repository say it is untrusted, and so does the
    /// server: a commit body must never read as an instruction to the agent.
    #[test]
    fn free_text_tools_warn_about_untrusted_content() {
        use rmcp::ServerHandler;
        let (_d, s) = fixture();
        let info = McpServer::new(s).get_info();
        assert!(info.instructions.is_some_and(|i| i.contains("untrusted")));
        for t in McpServer::tool_docs() {
            if ["read_file", "show_commit", "log", "blame"].contains(&t.name.as_str()) {
                assert!(t.description.contains("untrusted"), "{}", t.name);
            }
        }
    }

    #[test]
    fn etag_moves_with_cache_generation_and_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "one\n").expect("write");
        git(p, &["add", "."]);
        git(p, &["commit", "-q", "-m", "first"]);
        let s = ApiState::open(p).expect("open");
        let empty = s.etag().expect("tag");
        s.refresh_index().expect("index");
        let built = s.etag().expect("tag");
        assert_ne!(empty, built, "the cache filled with no ref moving");
        let bots = s.with_defaults(gitterrace_index::Options {
            include_bots: true,
            ..gitterrace_index::Options::default()
        });
        assert_ne!(built, bots.etag().expect("tag"), "other counting defaults");
    }

    /// The configuration reaches the answers: bots and excludes from `insights.*` and the
    /// repository's own `[repo.NAME] exclude`.
    #[test]
    fn configured_state_counts_with_the_config() {
        let (d, s) = fixture();
        let p = d.path();
        std::fs::write(p.join("a.txt"), "bot\n").expect("write");
        git(p, &["add", "."]);
        git(
            p,
            &[
                "-c",
                "user.name=dependabot[bot]",
                "-c",
                "user.email=bot@example.com",
                "commit",
                "-q",
                "-m",
                "chore: bump",
            ],
        );
        s.refresh_index().expect("index");
        let mut cfg = gitterrace_config::Config::default();
        cfg.insights.include_bots = true;
        cfg.repo.insert(
            s.repo.name(),
            gitterrace_config::RepoOverride {
                exclude: vec!["src/**".to_owned()],
                ..Default::default()
            },
        );
        let authors = |s: &ApiState| s.contributors(None).expect("contributors").authors.len();
        assert_eq!(authors(&s), 1, "bots hidden by default");
        // Settings are shared by every clone, so a router built before a reload sees it.
        let router_state = s.clone();
        let c = s.configured(&cfg);
        assert_eq!(authors(&c), 2, "insights.include_bots = true");
        assert_eq!(authors(&router_state), 2, "a reload reaches earlier clones");
        let hot = c
            .insight("hotspots", None, &crate::Params::default())
            .expect("hotspots");
        assert!(
            !hot.to_string().contains("src/b.rs"),
            "repo exclude ignored: {hot}"
        );
    }

    /// 5xx bodies carry a fixed message: the inner error names files under the repository.
    #[tokio::test]
    async fn server_errors_hide_the_inner_message() {
        let (d, s) = fixture();
        // Pages read the segment set: a manifest that cannot be read is an index error.
        let manifest = d.path().join(".git/gitterrace/MANIFEST");
        std::fs::remove_file(&manifest).expect("rm manifest");
        std::fs::create_dir(&manifest).expect("mkdir");
        let (st, v) = get(&s, "/repo").await;
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(v["error"]["code"], "index");
        assert_eq!(v["error"]["message"], crate::SERVER_ERROR_MESSAGE);
        let root = d.path().to_string_lossy().into_owned();
        assert!(!v.to_string().contains(&root), "{v}");
        let (st, v) = get(&s, "/blob?path=missing").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(
            v["error"]["message"]
                .as_str()
                .is_some_and(|m| m.contains("missing")),
            "4xx keep their text: {v}"
        );
    }
}
