//! Memory budget: a server holds at most 32 MB (the cache budget) + 16 MB more dirty memory
//! than it did just started, idle after indexing and after a sweep of every insights page and
//! API insight over every period, at HEAD and at a side branch HEAD lacks. Idle memory may grow
//! by at most `GROWTH_MB` from 2k commits to the largest repository.
//!
//! Measured on a child server process: `phys_footprint` (macOS `footprint`), `Private_Dirty`
//! of `/proc/PID/smaps_rollup` (Linux). File-backed page cache is not counted by either.
//!
//! Red on the in-memory implementation by design: it holds the parsed history. Runs in every
//! `cargo test --release` and in CI. `TEST_RUSTGIT=<path>` adds a real history.
#![cfg(not(debug_assertions))]
#![expect(clippy::expect_used, reason = "setup failures should panic")]

#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use gitterrace_testkit::matrix::PERIODS;
use gitterrace_testkit::synthetic::{self, Shape};

const BUDGET_MB: u64 = 32 + 16;
/// Most that idle memory may grow from 2k commits to the largest size measured.
const GROWTH_MB: u64 = 4;
const SERVE_VAR: &str = "TEST_MEMORY_SERVE";
/// Longest wait for an idle release after the server is ready.
const RELEASE_WAIT_SECS: u64 = 120;

/// The allocator the numbers depend on, as the shipped binary sets it at start
/// (`crates/gitterrace/src/alloc.rs`): jemalloc, every arena handing freed pages back at once.
/// The child server here starts from `serve_in`, not that `main`, so it applies the same
/// setting, then this asserts the decay in effect. A run under another allocator or decay
/// measures retained pages instead of held memory, so it fails here instead of reporting.
fn apply_and_assert_allocator() {
    use tikv_jemalloc_ctl::{Access, AsName};
    let version: &str = b"version\0"
        .name()
        .read()
        .expect("jemalloc is the global allocator");
    for key in ["dirty_decay_ms", "muzzy_decay_ms"] {
        for at in ["arenas", "arena.0"] {
            let name = format!("{at}.{key}\0");
            name.as_str().name().write(0_isize).expect("set decay");
            let ms: isize = name.as_str().name().read().expect("read decay");
            assert_eq!(
                ms, 0,
                "jemalloc {version}: {at}.{key} is {ms}, the server sets 0"
            );
        }
    }
}

/// Inside the child: serve the repository named by `TEST_MEMORY_SERVE=<addr> <path>`.
fn serve_if_child() -> bool {
    let Ok(v) = std::env::var(SERVE_VAR) else {
        return false;
    };
    apply_and_assert_allocator();
    let (addr, path) = v.split_once(' ').expect("addr and path");
    let addr: SocketAddr = addr.parse().expect("addr");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(gitterrace_web::serve_in(
        vec![PathBuf::from(path)],
        addr,
        None,
        gitterrace_config::Config::default(),
        None,
        None,
    ))
    .expect("serve");
    true
}

struct Server {
    child: Child,
    addr: SocketAddr,
}

impl Server {
    fn start(repo: &Path, test: &str) -> Self {
        let addr = std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .expect("free port");
        let child = Command::new(std::env::current_exe().expect("exe"))
            .args([test, "--exact", "--nocapture", "--include-ignored"])
            .env(SERVE_VAR, format!("{addr} {}", repo.display()))
            .spawn()
            .expect("spawn server");
        let s = Self { child, addr };
        let end = Instant::now() + Duration::from_secs(30);
        while TcpStream::connect(addr).is_err() {
            assert!(Instant::now() < end, "server did not start");
            gitterrace_testkit::wait::pause_blocking(Duration::from_millis(50));
        }
        s
    }

    /// GET `path` under the lone repository; returns status and body.
    fn get(&self, path: &str) -> (u16, String) {
        let mut s = TcpStream::connect(self.addr).expect("connect");
        s.set_read_timeout(Some(Duration::from_secs(600)))
            .expect("timeout");
        write!(
            s,
            "GET {path} HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n",
            self.addr.port()
        )
        .expect("send");
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).expect("read");
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status = text
            .split(' ')
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        (status, text)
    }

    /// The `/r/NAME` prefix `/` forwards to.
    fn base(&self) -> String {
        let (_, head) = self.get("/");
        head.lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("location")
                    .then(|| v.trim().trim_end_matches('/').to_owned())
            })
            .expect("/ forwards to the repository")
    }

    fn wait_ready(&self, base: &str) {
        let end = Instant::now() + Duration::from_secs(3600);
        loop {
            let (_, body) = self.get(&format!("{base}/stats"));
            if !body.contains("data-progress") {
                return;
            }
            assert!(Instant::now() < end, "index never became ready");
            gitterrace_testkit::wait::pause_blocking(Duration::from_millis(500));
        }
    }

    /// Waits until the server has released its idle caches at least once more than `before`,
    /// as `/api/progress` counts them. A release that never comes fails here: an idle server
    /// that keeps its caches is the red this test exists for, not a reading to take anyway.
    fn wait_released(&self, base: &str, before: u64) {
        // The release runs one idle period (30 s) after the last read, checked every quarter
        // period: twice that and a margin is a server that does not release.
        let end = Instant::now() + Duration::from_secs(RELEASE_WAIT_SECS);
        loop {
            if self.releases(base) > before {
                return;
            }
            assert!(
                Instant::now() < end,
                "no idle release within {RELEASE_WAIT_SECS} s: the server keeps its caches"
            );
            gitterrace_testkit::wait::pause_blocking(Duration::from_millis(500));
        }
    }

    /// Idle releases so far, from `/api/progress`.
    fn releases(&self, base: &str) -> u64 {
        let (_, body) = self.get(&format!("{base}/api/progress"));
        let json = body.split("\r\n\r\n").nth(1).unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(json.trim()).expect("progress json");
        v.get("releases")
            .and_then(serde_json::Value::as_u64)
            .expect("progress has a release count")
    }

    /// Dirty memory of the server in bytes.
    fn dirty(&self) -> u64 {
        dirty_of(self.child.id())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // A server that already exited is fine; one that cannot be killed fails the test.
        if self.child.try_wait().ok().flatten().is_none() {
            self.child.kill().expect("kill server");
            self.child.wait().expect("wait server");
        }
    }
}

#[cfg(target_os = "macos")]
fn dirty_of(pid: u32) -> u64 {
    let out = Command::new("footprint")
        .args(["-p", &pid.to_string()])
        .output()
        .expect("run footprint");
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .find(|l| l.contains("Footprint:"))
        .unwrap_or_else(|| panic!("no footprint line: {text}"));
    let field = line.split("Footprint:").nth(1).expect("value").trim();
    let mut parts = field.split_whitespace();
    let n: f64 = parts.next().and_then(|n| n.parse().ok()).expect("number");
    let unit = parts.next().unwrap_or("B");
    let mul = match unit {
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (n * mul) as u64
}

#[cfg(target_os = "linux")]
fn dirty_of(pid: u32) -> u64 {
    let text = std::fs::read_to_string(format!("/proc/{pid}/smaps_rollup")).expect("smaps_rollup");
    text.lines()
        .filter(|l| l.starts_with("Private_Dirty:"))
        .filter_map(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok())
        .sum::<u64>()
        * 1024
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn dirty_of(_pid: u32) -> u64 {
    0
}

/// Every insights page under every period (languages included), every API insight, then the
/// contributors endpoint. A page that answers non-200 fails: a sweep of errors proves nothing.
fn sweep(s: &Server, base: &str, side: usize) {
    let names: Vec<&str> = gitterrace_api::insight_list().map(|(n, _)| n).collect();
    // A side branch past the fresh-commit cap is refused, not loaded: the API answers 409
    // `needs_reindex`, the insights pages a 200 saying so. Under the cap every answer is a 200.
    let far = side > gitterrace_index::feed::FRESH_CAP;
    let api = |path: &str, rev: &str| {
        let (status, body) = s.get(path);
        if far && !rev.is_empty() {
            assert!(
                status == 409 && body.contains("needs_reindex"),
                "{path}: {status}"
            );
        } else {
            assert_eq!(status, 200, "{path}");
        }
    };
    // HEAD, then the side branch, whose history HEAD lacks: a revision other than the one
    // indexed for HEAD must also stay inside the budget. `side` 0 means no such branch.
    let revs: &[&str] = if side == 0 { &[""] } else { &["", "&rev=side"] };
    for &rev in revs {
        for period in PERIODS {
            for p in gitterrace_web::WARM_PAGES {
                let path = format!("{base}{p}?period={period}{rev}");
                let (status, body) = s.get(&path);
                assert_eq!(status, 200, "{path}");
                if far && !rev.is_empty() && p.starts_with("/stats") {
                    let what = if body.contains("data-progress") {
                        "the indexing page"
                    } else {
                        "another page"
                    };
                    assert!(
                        body.contains("need a reindex"),
                        "{path}: no reindex notice, {what}"
                    );
                }
            }
            for n in &names {
                api(
                    &format!("{base}/api/v1/insights/{n}?period={period}{rev}"),
                    rev,
                );
            }
        }
        api(
            &format!("{base}/api/v1/contributors?{}", rev.trim_start_matches('&')),
            rev,
        );
    }
}

const MB: u64 = 1024 * 1024;

/// Returns (just started, idle, after sweep) in bytes. The caller asserts, after every size
/// is measured, so one red size never hides the others.
fn measure(repo: &Path, test: &str, side: usize) -> (u64, u64, u64) {
    // "Just started" is a server on a one-commit repository: on `repo` itself indexing begins
    // at once, so its first reading would already hold the build.
    let start = {
        let tiny = tempfile::tempdir().expect("tmp");
        synthetic::build(
            tiny.path(),
            Shape {
                commits: 1,
                ..Shape::sized(1)
            },
        );
        let t = Server::start(tiny.path(), test);
        let b = t.base();
        t.wait_ready(&b);
        t.dirty()
    };
    let s = Server::start(repo, test);
    let base = s.base();
    s.wait_ready(&base);
    // Idle means after the server released what it built: wait for that release, not a time.
    s.wait_released(&base, 0);
    let idle = s.dirty();
    sweep(&s, &base, side);
    let swept = s.dirty();
    // The measurements, for the record of a red run: stderr is the test's report channel.
    writeln!(
        std::io::stderr(),
        "{}: start {} MB, idle {} MB, after sweep {} MB",
        repo.display(),
        start / MB,
        idle / MB,
        swept / MB
    )
    .expect("stderr");
    (start, idle, swept)
}

/// Red until the pages read from the segment store on disk: today the server holds the
/// parsed history, 167 MB above a fresh one at 20k commits. Release builds only, debug is too
/// slow to index 100k commits. Runs that must not wait on it filter it out by name.
#[test]
fn memory_stays_in_budget_at_every_size() {
    if serve_if_child() {
        return;
    }
    apply_and_assert_allocator();
    if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    // The side branch: under the fresh-commit cap at 2k (read from git directly), over it at
    // 20k and 100k (the path that must refuse, never load every record).
    let over = gitterrace_index::feed::FRESH_CAP + 500;
    // `TEST_MEMORY_SIZES=2000,20000` measures a subset, for a quick look; CI runs all three.
    let only: Option<Vec<usize>> = std::env::var("TEST_MEMORY_SIZES")
        .ok()
        .map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect());
    let mut got = Vec::new();
    for (n, side) in [(2_000, 300), (20_000, over), (100_000, over)] {
        if only.as_ref().is_some_and(|o| !o.contains(&n)) {
            continue;
        }
        let repo = dir.path().join(format!("r{n}"));
        synthetic::build(
            &repo,
            Shape {
                side,
                ..Shape::sized(n)
            },
        );
        let ahead = gitterrace_testkit::git(&repo, &["rev-list", "--count", "main..side"]);
        assert_eq!(ahead, side.to_string(), "side branch length");
        got.push((
            n,
            measure(&repo, "memory_stays_in_budget_at_every_size", side),
        ));
    }
    let mut bad = Vec::new();
    for &(n, (start, idle, swept)) in &got {
        for (what, v) in [("idle", idle), ("after sweep", swept)] {
            if v > start + BUDGET_MB * MB {
                bad.push(format!(
                    "{n} commits, {what}: {} MB above the just-started {} MB, budget {BUDGET_MB} \
                    MB",
                    v.saturating_sub(start) / MB,
                    start / MB
                ));
            }
        }
    }
    // Growth with history, idle and after the sweep (HEAD and the side branch): what a server
    // keeps per commit. The budget alone allows 48 MB at any size, which hides a per-commit
    // cost until a large repository (0.7 KB per commit is 12 MB at 20k but 1 GB on linux).
    if let (Some(&(_, (_, idle0, swept0))), Some(&(n, (_, idle1, swept1)))) =
        (got.first(), got.last())
        && n > 2_000
    {
        for (what, small, large) in [("idle", idle0, idle1), ("after sweep", swept0, swept1)] {
            if large > small + GROWTH_MB * MB {
                bad.push(format!(
                    "{what} grows with history: {} MB at {n} commits, {} MB at 2000, more than \
                     {GROWTH_MB} MB apart",
                    large / MB,
                    small / MB
                ));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Opt-in on a real history: `TEST_RUSTGIT=<path>`. A no-op without it.
#[test]
fn memory_stays_in_budget_on_rust() {
    if serve_if_child() {
        return;
    }
    if let Some(repo) = std::env::var_os("TEST_RUSTGIT") {
        measure(Path::new(&repo), "memory_stays_in_budget_on_rust", 0);
    }
}

/// Idle memory is flat in the size of the tree, not only the history: 300 commits over 1k and
/// over `TEST_MEMORY_WIDE` (default 50k) tracked files, read after the idle release. A cache
/// sized by the tree (the HEAD index a blame or diff loads, a path list) that survives the
/// release shows here and not in the commit sizes. The release gate runs 100k files.
#[test]
fn idle_memory_is_flat_in_tree_size() {
    if serve_if_child() {
        return;
    }
    apply_and_assert_allocator();
    if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
        return;
    }
    let wide: usize = std::env::var("TEST_MEMORY_WIDE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000);
    let dir = tempfile::tempdir().expect("tmp");
    let mut idle = Vec::new();
    for files in [1_000, wide] {
        let repo = dir.path().join(format!("w{files}"));
        synthetic::build(&repo, Shape::wide(files));
        let s = Server::start(&repo, "idle_memory_is_flat_in_tree_size");
        let base = s.base();
        s.wait_ready(&base);
        // A blame and a tree page are what a visitor reads first; both load tree-sized state.
        let file = gitterrace_testkit::git(&repo, &["ls-files"])
            .lines()
            .next()
            .expect("a tracked file")
            .to_owned();
        let before = s.releases(&base);
        for path in [
            format!("{base}/blame/HEAD/{file}"),
            format!("{base}/tree/HEAD"),
        ] {
            assert_eq!(s.get(&path).0, 200, "{path}");
        }
        s.wait_released(&base, before);
        let v = s.dirty();
        writeln!(
            std::io::stderr(),
            "{files} files: idle after release {} MB",
            v / MB
        )
        .expect("stderr");
        idle.push((files, v));
    }
    let (small, large) = (idle[0].1, idle[1].1);
    assert!(
        large <= small + GROWTH_MB * MB,
        "idle grows with the tree: {} MB at {wide} files, {} MB at 1000, more than {GROWTH_MB} MB \
        apart",
        large / MB,
        small / MB
    );
}

#[path = "memory_budget/insights.rs"]
mod insights;
#[path = "memory_budget/renames.rs"]
mod renames;
