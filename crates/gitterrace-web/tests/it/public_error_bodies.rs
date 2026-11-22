//! A public server whose insights cache is broken answers 5xx, and no body it sends, page or
//! API, names the folder the repository lives in.

use gitterrace_testkit::{Fixture, links};

const HOST: &str = "git.example.com";
const MARKER: &str = "host-folder-marker";

#[tokio::test]
async fn public_error_bodies_never_name_host_folders() {
    let dir = tempfile::tempdir().expect("tmp");
    // The marker is a parent folder: the repository's own folder name is its public name.
    let p = Fixture::standard(&dir.path().join(MARKER).join("repo")).path().to_owned();
    let mut cfg = gitterrace_config::Config::default();
    cfg.server.is_public = true;
    cfg.server.hosts = vec![HOST.to_owned()];
    // A public server serves the API only when asked; this test reads its error bodies too.
    cfg.features.api_enabled = Some(true);
    let router = gitterrace_web::app_router_with(&p, cfg).expect("router");
    // A directory where the cache expects a file makes every cache read fail, and the error
    // text of that failure names the full path.
    let manifest = p.join(".git/gitterrace/MANIFEST");
    std::fs::remove_file(&manifest).ok();
    std::fs::create_dir_all(&manifest).expect("mkdir");
    let seeds: Vec<String> = ["/", "/r/repo/", "/r/repo/stats", "/r/repo/api/v1/"]
        .into_iter()
        .map(str::to_owned)
        .chain(gitterrace_api::ENDPOINTS.iter().map(|e| {
            // An endpoint refuses a parameter it does not list, before any cache read.
            let query =
                if e.params.iter().any(|p| p.name == "path") { "?path=src/a.rs" } else { "" };
            format!(
                "/r/repo/api/v1{}{query}",
                e.path.replace("{id}", "HEAD").replace("{name}", "summary")
            )
        }))
        .collect();
    let mut bad = Vec::new();
    let mut server_errors = 0;
    crate::common::crawl(&router, HOST, seeds, 200, links, |u, s, body| {
        if s >= 500 {
            server_errors += 1;
        }
        if body.contains(MARKER) {
            bad.push(format!("{u} ({s})"));
        }
    })
    .await;
    assert!(server_errors > 0, "the broken cache produced no 5xx: the guard tests nothing");
    assert!(bad.is_empty(), "bodies name the host folder:\n{}", bad.join("\n"));
}
