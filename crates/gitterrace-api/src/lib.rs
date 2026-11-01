//! Read-only JSON API (axum router) and MCP server over gitterrace-git and gitterrace-index.
//!
//! Both surfaces call the same [`ApiState`] methods, so they return the same data.

#![allow(clippy::unused_async_trait_impl)]

mod core;
mod endpoints;
mod error;
mod http;
mod insights;
mod json;
mod mcp;

pub use core::{ApiState, MAX_BLAME_LINES, MAX_BLOB_BYTES, MAX_BLOB_LINES, MAX_PAGE};
pub use endpoints::{ENDPOINTS, Endpoint, Param};
pub use error::ApiError;
pub use http::router;
pub use insights::{INSIGHTS, Insight, Params as InsightParams};
pub use mcp::{McpServer, Param as ToolParam, ToolDoc, serve_mcp};

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::{ApiState, McpServer, router};

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(["-c", "user.name=T", "-c", "user.email=t@x.io"])
            .args(args)
            .current_dir(dir)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git {args:?} failed");
    }

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
    async fn every_insight_answers() {
        let (_d, s) = fixture();
        for i in crate::INSIGHTS {
            let (st, v) = get(&s, &format!("/insights/{}", i.name)).await;
            assert_eq!(st, StatusCode::OK, "{}", i.name);
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
        let v = s.blob(None, "min.js", None, None).expect("blob");
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
        let names = McpServer::tool_names();
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
}
