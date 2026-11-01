//! `git terrace open <link>`: find a local clone a `gitterrace://` or `/go` link names, make
//! sure a server for it is listening, and open the browser at the page. A link comes from
//! outside, so it is parsed strictly, looked up only among clones the user served or listed
//! in `links.roots`, and an unknown id ends here: no clone, no network.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Context;
use gitterrace_actions::Link;
use gitterrace_git::Repo;

/// How long a freshly started server gets to answer. Opening the index of a large repository
/// takes a second or two; past this the browser opens anyway and shows its own error.
const READY_WAIT: Duration = Duration::from_secs(8);
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Why `open` stops before a browser opens. `Unknown` exits 2.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OpenError {
    #[error("{0}")]
    BadLink(#[from] gitterrace_actions::LinkError),
    #[error(
        "no local clone of {0} known. Run `git terrace` in it once, or list its folder in links.roots"
    )]
    Unknown(String),
}

/// The clone a link resolved to and the `/go` path to open on its server.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) clone: PathBuf,
    /// `/go/...` for the commit (and path), or `/` when the clone lacks the commit.
    pub(crate) path: String,
    /// A line for the user when the page is not the one the link names.
    pub(crate) note: Option<String>,
}

/// Where a link points, among `known` clones (registry `(id, path)` pairs) and repositories
/// found under `roots`. A registry id only picks candidates, since reading an id walks the whole
/// history (seconds on a large clone); every candidate's id is read again before it is used.
pub(crate) fn resolve(
    input: &str,
    known: &[(String, PathBuf)],
    roots: &[PathBuf],
    depth: usize,
    skip: &[String],
) -> Result<Target, OpenError> {
    let link = Link::parse(input.trim())?;
    let found = gitterrace_web::discover_repos(roots, depth, skip);
    let clones: Vec<Repo> = known
        .iter()
        .filter(|(id, _)| *id == link.id)
        .map(|(_, p)| p)
        .chain(
            found
                .iter()
                .filter(|p| !roots.contains(p) || p.join(".git").exists()),
        )
        .filter_map(|p| Repo::discover(p).ok())
        .filter(|r| r.id().as_deref() == Some(link.id.as_str()))
        .collect();
    let Some(first) = clones.first() else {
        return Err(OpenError::Unknown(link.id));
    };
    for r in &clones {
        let Some(full) = r
            .resolve(&link.sha)
            .ok()
            .filter(|f| f.starts_with(&link.sha))
        else {
            continue;
        };
        let mut l = Link {
            sha: full.clone(),
            ..link
        };
        let mut note = None;
        if !r.has(&full, l.path.as_deref()) {
            note = Some(format!(
                "{} is not in commit {}; opening the commit",
                l.path.as_deref().unwrap_or_default(),
                gitterrace_git::short_id(&full)
            ));
            l.path = None;
            l.lines = None;
        }
        return Ok(Target {
            clone: r.root(),
            path: l.go(""),
            note,
        });
    }
    Ok(Target {
        clone: first.root(),
        path: "/".to_owned(),
        note: Some(format!(
            "commit {} is not in {} (the clone may be behind), opening the overview",
            link.sha,
            first.root().display()
        )),
    })
}

#[expect(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// Runs the subcommand: resolve, find or start a server, then open or print the URL.
pub(crate) fn run(
    input: &str,
    print: bool,
    loaded: &gitterrace_config::Loaded,
    config_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let cfg = &loaded.config;
    let state = gitterrace_web::state_dir();
    let known: Vec<(String, PathBuf)> = state
        .as_deref()
        .map(gitterrace_web::registry_candidates)
        .transpose()
        .context("reading the served-repositories registry")?
        .unwrap_or_default()
        .into_iter()
        .map(|e| (e.id, e.path))
        .collect();
    let roots: Vec<PathBuf> = cfg.links.roots.iter().map(|r| expand(r)).collect();
    let target = resolve(input, &known, &roots, cfg.multi.depth, &cfg.multi.skip_dirs)?;
    if let Some(n) = &target.note {
        eprintln!("gitterrace: {n}");
    }
    let configured: SocketAddr = cfg.server.addr.parse().context("server.addr")?;
    let addr = ensure_server(&target.clone, configured, state.as_deref(), config_dir)?;
    let url = format!("http://{addr}{}", target.path);
    if print {
        println!("{url}");
        return Ok(());
    }
    browse(&url)
}

#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// The address of a server serving `clone`: the configured one when it already does, else a
/// new detached server, on the configured address when free and on a free port when another
/// server holds it.
fn ensure_server(
    clone: &Path,
    configured: SocketAddr,
    state: Option<&Path>,
    config_dir: Option<&Path>,
) -> anyhow::Result<SocketAddr> {
    let addr = loopback(configured);
    let repo = Repo::discover(clone).context("opening the clone")?;
    let id = repo.id().unwrap_or_default();
    let head = repo.resolve("HEAD").context("reading HEAD")?;
    let probe = format!("/go/{id}/{head}");
    if serves(addr, &probe) {
        return Ok(addr);
    }
    let addr = if TcpListener::bind(addr).is_ok() {
        addr
    } else {
        let l = TcpListener::bind((addr.ip(), 0)).context("finding a free port")?;
        l.local_addr()?
    };
    let log = match state {
        Some(d) => {
            std::fs::create_dir_all(d)?;
            Stdio::from(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(d.join("server.log"))
                    .context("opening server.log")?,
            )
        }
        None => Stdio::null(),
    };
    let mut cmd = Command::new(std::env::current_exe().context("finding this program")?);
    cmd.arg(clone).arg("--addr").arg(addr.to_string());
    if let Some(d) = config_dir {
        cmd.arg("--config").arg(d);
    }
    cmd.arg("serve")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log);
    #[cfg(unix)]
    {
        // Its own process group, so closing the terminal that ran `open` leaves it running.
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().context("starting the server")?;
    let start = Instant::now();
    while start.elapsed() < READY_WAIT {
        if serves(addr, &probe) {
            return Ok(addr);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("gitterrace: the server at {addr} is slow to start; see server.log");
    Ok(addr)
}

/// Whether `addr` answers `GET path` with a redirect: `/go` redirects only for a repository
/// it serves, so a server for another repository (or not gitterrace at all) says no.
fn serves(addr: SocketAddr, path: &str) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return false;
    };
    if s.set_read_timeout(Some(PROBE_TIMEOUT)).is_err() {
        return false;
    }
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 12];
    s.read_exact(&mut head).is_ok() && head.starts_with(b"HTTP/1.1 302")
}

/// A server bound to every interface is reached over loopback.
fn loopback(a: SocketAddr) -> SocketAddr {
    if a.ip().is_unspecified() {
        let ip = if a.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        };
        SocketAddr::new(ip, a.port())
    } else {
        a
    }
}

/// `~/x` against `$HOME`; anything else as written.
fn expand(p: &str) -> PathBuf {
    let rest = p
        .strip_prefix("~/")
        .or_else(|| p.strip_prefix("~\\").filter(|_| cfg!(windows)));
    match (rest, gitterrace_config::home_var()) {
        (Some(rest), Some(h)) => PathBuf::from(h).join(rest),
        _ => PathBuf::from(p),
    }
}

/// The platform's URL opener. The URL is built here from validated parts and starts with
/// `http://`, so it can never read as an option.
fn browse(url: &str) -> anyhow::Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if std::fs::read_to_string("/proc/version")
        .is_ok_and(|v| v.to_ascii_lowercase().contains("microsoft"))
    {
        "wslview"
    } else {
        "xdg-open"
    };
    let ok = Command::new(opener)
        .arg(url)
        .status()
        .with_context(|| format!("running {opener}"))?
        .success();
    anyhow::ensure!(ok, "{opener} could not open {url}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;
    use std::path::Path;

    use crate::open::{OpenError, resolve};

    /// A repository with `src/a.rs`; returns (root, id, HEAD).
    fn repo(dir: &Path) -> (std::path::PathBuf, String, String) {
        std::fs::create_dir_all(dir.join("src")).expect("dir");
        git(dir, &["init", "-q"]);
        std::fs::write(dir.join("src/a.rs"), "a\nb\nc\n").expect("w");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", "root"]);
        let head = git(dir, &["rev-parse", "HEAD"]);
        let root = git(dir, &["rev-list", "--max-parents=0", "HEAD"]);
        (
            dir.canonicalize().expect("canon"),
            root[..12].to_owned(),
            head,
        )
    }

    #[test]
    fn resolves_registered_and_root_clones() {
        let t = tempfile::tempdir().expect("tmp");
        let (a, id, head) = repo(&t.path().join("a"));
        let link = format!("gitterrace://{id}/{}/src/a.rs#L2-L3", &head[..10]);
        let want = format!("/go/{id}/{head}/src/a.rs#L2-L3");

        let got = resolve(&link, &[(id.clone(), a.clone())], &[], 2, &[]).expect("registered");
        assert_eq!(
            (got.clone, got.path, got.note),
            (a.clone(), want.clone(), None)
        );

        let got = resolve(&link, &[], &[t.path().to_owned()], 2, &[]).expect("root");
        assert_eq!((got.clone, got.path), (a.clone(), want.clone()));

        let go = format!("http://127.0.0.1:9/go/{id}/{head}/src/a.rs#L2-L3");
        assert_eq!(
            resolve(&go, &[(id, a)], &[], 2, &[]).expect("go").path,
            want
        );
    }

    #[test]
    fn unknown_id_missing_commit_and_missing_path() {
        let t = tempfile::tempdir().expect("tmp");
        let (a, id, head) = repo(&t.path().join("a"));
        let other = format!("gitterrace://000000000000/{head}");
        assert!(matches!(
            resolve(
                &other,
                &[("000000000000".to_owned(), a.clone())],
                &[t.path().to_owned()],
                2,
                &[]
            ),
            Err(OpenError::Unknown(_))
        ));
        let behind = format!("gitterrace://{id}/{}", "f".repeat(40));
        let got = resolve(&behind, &[(id.clone(), a.clone())], &[], 2, &[]).expect("behind");
        assert_eq!(got.path, "/");
        assert!(got.note.is_some_and(|n| n.contains("behind")));
        let gone = format!("gitterrace://{id}/{head}/no/such.rs");
        let got = resolve(&gone, &[(id.clone(), a)], &[], 2, &[]).expect("gone");
        assert_eq!(got.path, format!("/go/{id}/{head}"));
        assert!(matches!(
            resolve(&format!("gitterrace://{id}/{head}/../x"), &[], &[], 2, &[]),
            Err(OpenError::BadLink(_))
        ));
    }
}
