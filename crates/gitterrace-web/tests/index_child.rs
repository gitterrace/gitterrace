//! A server's index update runs in a child process, so the build's peak (allocator metadata,
//! worker stacks) dies with it instead of staying with the server.
//!
//! The child is this test executable started again with the same arguments, as the binary
//! starts itself: the test calls `index_child` first, as `main` does, so the child runs the
//! update and exits there.

#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Index worker threads started in this process.
static WORKERS: AtomicUsize = AtomicUsize::new(0);

fn count(on: bool) {
    if on {
        WORKERS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Guard class: any index work done on the server's own threads, whether a new caller of
/// `Index::update` or a build step moved out of the child, starts a worker here.
#[tokio::test(flavor = "multi_thread")]
async fn the_server_builds_its_index_without_a_worker_of_its_own() {
    gitterrace_git::set_worker_hook(count);
    gitterrace_web::index_child();
    let dir = tempfile::tempdir().expect("tmp");
    let p = gitterrace_testkit::Fixture::init(&dir.path().join("r"))
        .write("a.txt", "one\n")
        .commit("feat: one")
        .write("a.txt", "two\n")
        .commit("fix: two")
        .path()
        .to_owned();
    let router = gitterrace_web::app_router(&p).expect("router");
    let body = gitterrace_testkit::wait::page_until(&router, "/api/progress", |b| {
        b.contains("\"ready\":true")
    })
    .await;
    assert!(
        body.contains("\"name\":\"stream\""),
        "steps of the child: {body}"
    );
    // The child's CPU and worker busy share cross the pipe: a busy share is only measured
    // where the workers ran, so one here came from the child.
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let steps = v["phases"].as_array().expect("phases");
    let stream = steps
        .iter()
        .find(|p| p["name"] == "stream")
        .expect("stream");
    assert!(
        stream["busy"].as_f64().is_some_and(|b| b > 0.0),
        "no worker busy share from the child: {body}"
    );
    #[cfg(unix)]
    assert!(
        stream["cpu_secs"].as_f64().is_some(),
        "no CPU seconds from the child: {body}"
    );
    assert_eq!(
        WORKERS.load(Ordering::Relaxed),
        0,
        "index workers ran in the server"
    );
}

/// Set in the re-executed test: it is the server whose index children hold.
#[cfg(unix)]
const INNER_VAR: &str = "TEST_INDEX_CHILD_INNER";

/// Guard class: any way taking a repository off leaves its index build running, whether the
/// child is not killed, not reaped, or killed after the reply. The removed repository's
/// child is held mid-build; once `DELETE /_repos` answers, its pid is gone and its cache
/// holds no file of that build.
#[cfg(unix)]
#[test]
fn a_removed_repository_leaves_no_index_child_behind() {
    // First, as in every test here: in an index child this runs the update and exits.
    gitterrace_web::index_child();
    if std::env::var_os(INNER_VAR).is_none() {
        let hold = tempfile::tempdir().expect("tmp");
        let status = std::process::Command::new(std::env::current_exe().expect("exe"))
            .args([
                "a_removed_repository_leaves_no_index_child_behind",
                "--exact",
                "--nocapture",
            ])
            .env(INNER_VAR, "1")
            .env("TEST_INDEX_CHILD_HOLD", hold.path())
            .status()
            .expect("spawn");
        assert!(status.success(), "inner run: {status}");
        return;
    }
    let hold = std::path::PathBuf::from(std::env::var_os("TEST_INDEX_CHILD_HOLD").expect("hold"));
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().canonicalize().expect("canon");
    let repos: Vec<_> = ["alpha", "beta"]
        .iter()
        .map(|n| {
            gitterrace_testkit::Fixture::init(&root.join(n))
                .write("a.txt", "one\n")
                .commit("feat: one")
                .path()
                .to_owned()
        })
        .collect();
    let cache = root.join("alpha/.git/gitterrace");
    let state = root.join("state");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve = gitterrace_web::serve_on(
        listener,
        repos,
        None,
        gitterrace_config::Config::default(),
        None,
        Some(state.clone()),
        &[],
    );
    rt.spawn(async move { drop(serve.await) });
    // The child holding alpha's cache, once it is mid-build.
    let mut pid = None;
    gitterrace_testkit::wait::until_blocking("alpha's index child holds", || {
        pid = std::fs::read_dir(&hold).ok().and_then(|d| {
            d.filter_map(Result::ok).find_map(|e| {
                let owner = std::fs::read(e.path()).ok()?;
                let p: i32 = e.file_name().to_str()?.parse().ok()?;
                (std::path::Path::new(std::ffi::OsStr::from_bytes(&owner)) == cache).then_some(p)
            })
        });
        pid.is_some()
    });
    let pid = rustix::process::Pid::from_raw(pid.expect("pid")).expect("nonzero pid");
    let before = files(&cache);
    gitterrace_testkit::wait::until_blocking("the add secret is published", || {
        gitterrace_web::add_token(&state, addr.port()).is_ok()
    });
    let token = gitterrace_web::add_token(&state, addr.port()).expect("token");
    let reply = delete(addr, &token, r#"{"name":"alpha"}"#).expect("DELETE /_repos");
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(
        rustix::process::test_kill_process(pid).is_err(),
        "the index child of a removed repository still runs"
    );
    assert_eq!(
        files(&cache),
        before,
        "the stopped build wrote into the cache"
    );
    assert!(
        !cache.join("MANIFEST").exists(),
        "the stopped build published a set"
    );
    // Beta's child holds forever: never wait for its update. It ends with this process,
    // whose exit closes its stdin.
    rt.shutdown_background();
}

/// File names in `dir` with their sizes.
#[cfg(unix)]
fn files(dir: &std::path::Path) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = std::fs::read_dir(dir)
        .map(|d| {
            d.filter_map(Result::ok)
                .map(|e| {
                    let len = e.metadata().map(|m| m.len()).unwrap_or_default();
                    (e.file_name().to_string_lossy().into_owned(), len)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(unix)]
fn delete(addr: std::net::SocketAddr, token: &str, body: &str) -> std::io::Result<String> {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(addr)?;
    write!(
        s,
        "DELETE /_repos HTTP/1.1\r\nHost: {addr}\r\n{}: {token}\r\nContent-Type: \
        application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        gitterrace_web::ADD_TOKEN_HEADER,
        body.len()
    )?;
    let mut buf = String::new();
    s.read_to_string(&mut buf)?;
    Ok(buf)
}
