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
    let addr = ensure_server(&target.clone, configured, state.as_deref(), &mut |c, a| {
        spawn_server(c, a, state.as_deref(), config_dir)
    })?;
    if let Some(hint) = std::env::current_exe()
        .ok()
        .and_then(|e| crate::handler::stale_hint(&e))
    {
        eprintln!("gitterrace: {hint}");
    }
    let url = format!("http://{addr}{}", target.path);
    if print {
        println!("{url}");
        return Ok(());
    }
    browse(&url)
}

/// What answers on an address.
#[derive(Debug, PartialEq, Eq)]
enum Listener {
    Free,
    Gitterrace,
    Other,
}

#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// A second `git terrace` on an address a local gitterrace server already holds hands its
/// repositories to that server and exits, instead of failing to bind. `Ok(true)` when it did;
/// `Ok(false)` leaves the caller to bind (and report) as usual, and nothing was added. Every
/// root is opened first, so a bad one stops before any is added. Once adding starts, a refusal
/// is an error: falling through to bind would only fail on the held address, half joined.
pub(crate) fn join_running(
    addr: SocketAddr,
    roots: &[PathBuf],
    state: Option<&Path>,
) -> anyhow::Result<bool> {
    let addr = loopback(addr);
    if roots.is_empty() || identify(addr, state) != Listener::Gitterrace {
        return Ok(false);
    }
    let mut repos = Vec::with_capacity(roots.len());
    for root in roots {
        let repo = Repo::discover(root).with_context(|| format!("opening {}", root.display()))?;
        repos.push(repo.root());
    }
    for (root, repo) in roots.iter().zip(&repos) {
        let base = add_repo(addr, state, repo).map_err(|why| {
            anyhow::anyhow!(
                "the server on {addr} could not add {}: {why}",
                root.display()
            )
        })?;
        eprintln!("gitterrace: added to the server already running: http://{addr}{base}");
    }
    Ok(true)
}

#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// The address of a server serving `clone`. A gitterrace server on the configured address that
/// serves it is joined; one that does not is asked to add it (`POST /_repos` with the secret
/// it wrote to `state`) and then joined. Only when that fails, or a foreign program holds the
/// address, does the clone get its own server on a free port, with the reason. A running server
/// is never replaced. Only a free configured address gets the new server. `start` launches a
/// server for the clone on the address it is given.
fn ensure_server(
    clone: &Path,
    configured: SocketAddr,
    state: Option<&Path>,
    start: &mut dyn FnMut(&Path, SocketAddr) -> anyhow::Result<()>,
) -> anyhow::Result<SocketAddr> {
    let addr = loopback(configured);
    let repo = Repo::discover(clone).context("opening the clone")?;
    let id = repo.id().unwrap_or_default();
    let head = repo.resolve("HEAD").context("reading HEAD")?;
    let probe = format!("/go/{id}/{head}");
    let addr = match identify(addr, state) {
        Listener::Gitterrace if serves(addr, &probe) => return Ok(addr),
        Listener::Free => addr,
        found => {
            let what = if found == Listener::Gitterrace {
                match add_repo(addr, state, &repo.root()) {
                    Ok(_) if serves(addr, &probe) => return Ok(addr),
                    Ok(_) => "a gitterrace server that added this repository but does not open it"
                        .to_owned(),
                    Err(why) => format!(
                        "a gitterrace server that does not serve this repository and could not add it ({why})"
                    ),
                }
            } else {
                "another program".to_owned()
            };
            let l = TcpListener::bind((addr.ip(), 0)).context("finding a free port")?;
            let other = l.local_addr()?;
            drop(l);
            eprintln!(
                "gitterrace: {addr} is held by {what}; it is left running and this repository gets its own server at {other}. Restart the server on {addr} with this repository added to see it there."
            );
            other
        }
    };
    start(clone, addr)?;
    let began = Instant::now();
    while began.elapsed() < READY_WAIT {
        if serves(addr, &probe) {
            return Ok(addr);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("gitterrace: the server at {addr} is slow to start. See server.log");
    Ok(addr)
}

/// Asks the gitterrace server on `addr` to add the repository at `root` (canonical). Already
/// served (409) counts as added.
/// The `/r/NAME/` prefix the server serves `root` under, new or already there.
fn add_repo(addr: SocketAddr, state: Option<&Path>, root: &Path) -> Result<String, String> {
    let body = serde_json::json!({ "path": root });
    match secret_request(addr, state, "POST", &body)? {
        (201 | 409, v) => Ok(v
            .get("base")
            .and_then(|b| b.as_str().map(str::to_owned))
            .unwrap_or_else(|| "/".to_owned())),
        (404 | 405, _) => Err("it predates adding repositories or does not allow it".to_owned()),
        (other, v) => Err(format!("HTTP {other}: {}", why(&v))),
    }
}

#[expect(
    clippy::print_stderr,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// `git terrace remove`: the current repository when no target is given.
pub(crate) fn remove_cmd(addr: SocketAddr, target: Option<&str>) -> anyhow::Result<()> {
    let state = gitterrace_web::state_dir();
    let said = remove_running(addr, target.unwrap_or("."), state.as_deref())?;
    eprintln!("gitterrace: {said}");
    Ok(())
}

/// What `git terrace remove` did, for the one line it prints.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Removed {
    Done(String),
    NotServed,
}

/// `git terrace remove [PATH|NAME]`: finds and proves the server as [`join_running`] does, then
/// asks it to take the repository off (`DELETE /_repos`). A target that is an existing path
/// names a repository by its top folder, anything else is a URL name. No server, a refusal or
/// the last repository is an error; one the server does not serve is a plain answer.
pub(crate) fn remove_running(
    addr: SocketAddr,
    target: &str,
    state: Option<&Path>,
) -> anyhow::Result<String> {
    let addr = loopback(addr);
    let body = if Path::new(target).exists() {
        let repo =
            Repo::discover(Path::new(target)).with_context(|| format!("opening {target}"))?;
        let root = repo.root();
        serde_json::json!({ "path": root.canonicalize().unwrap_or(root) })
    } else {
        serde_json::json!({ "name": target })
    };
    match identify(addr, state) {
        Listener::Gitterrace => {}
        Listener::Free => anyhow::bail!("no gitterrace server is running on {addr}"),
        Listener::Other => anyhow::bail!(
            "{addr} is held by a program that is not a gitterrace server of this user, or one started with --public"
        ),
    }
    Ok(
        match remove_repo(addr, state, &body)
            .map_err(|e| anyhow::anyhow!("the server on {addr} did not remove {target}: {e}"))?
        {
            Removed::Done(name) => {
                format!("removed {name} from the server on {addr}; its index stays on disk")
            }
            Removed::NotServed => format!("{target} is not served on {addr}"),
        },
    )
}

fn remove_repo(
    addr: SocketAddr,
    state: Option<&Path>,
    body: &serde_json::Value,
) -> Result<Removed, String> {
    match secret_request(addr, state, "DELETE", body)? {
        (200, v) => Ok(Removed::Done(
            v.get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default()
                .to_owned(),
        )),
        (404, v) if v.get("error").is_some() => Ok(Removed::NotServed),
        (404 | 405, _) => Err("it predates removing repositories".to_owned()),
        (other, v) => Err(format!("HTTP {other}: {}", why(&v))),
    }
}

/// The `error` of a JSON reply, or the reply itself.
fn why(v: &serde_json::Value) -> String {
    v.get("error")
        .or_else(|| Some(v).filter(|v| v.is_string()))
        .and_then(|e| e.as_str().map(str::to_owned))
        .unwrap_or_else(|| v.to_string().chars().take(200).collect())
}

/// One `/_repos` request carrying the secret from the 0600 file the server on `addr` wrote to
/// `state`, so only its own user can ask. The status and the JSON body (`null` when none).
fn secret_request(
    addr: SocketAddr,
    state: Option<&Path>,
    method: &str,
    body: &serde_json::Value,
) -> Result<(u16, serde_json::Value), String> {
    let state = state.ok_or("no state folder to read its secret from")?;
    let token = gitterrace_web::add_token(state, addr.port())
        .map_err(|e| format!("its secret is unreadable: {e}"))?;
    let body = body.to_string();
    let mut s = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).map_err(|e| e.to_string())?;
    // Building the repository's router opens its index; give it longer than a probe.
    s.set_read_timeout(Some(READY_WAIT))
        .map_err(|e| e.to_string())?;
    let req = format!(
        "{method} /_repos HTTP/1.1\r\nHost: {addr}\r\n{}: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        gitterrace_web::ADD_TOKEN_HEADER,
        body.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    // A timeout after a partial read still leaves the status line to judge.
    let _partial = s.take(64 * 1024).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let Some(status) = text.get(9..12).and_then(|c| c.parse::<u16>().ok()) else {
        return Err("no answer".to_owned());
    };
    let reply = text.split("\r\n\r\n").nth(1).unwrap_or_default().trim();
    let v =
        serde_json::from_str(reply).unwrap_or_else(|_| serde_json::Value::String(reply.to_owned()));
    Ok((status, v))
}

/// Starts a detached `git-terrace <clone> --addr <addr> serve`, logging to `server.log`.
fn spawn_server(
    clone: &Path,
    addr: SocketAddr,
    state: Option<&Path>,
    config_dir: Option<&Path>,
) -> anyhow::Result<()> {
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
    Ok(())
}

/// Whether a gitterrace server, nothing, or something else listens on `addr`. A gitterrace
/// server proves it holds the add secret from our 0600 token file: it answers a fresh nonce
/// with `sha256(secret:nonce)`. A listener that cannot (another program, another user's
/// server, a stale token) is `Other`, so it is never sent a repository path.
fn identify(addr: SocketAddr, state: Option<&Path>) -> Listener {
    let Ok(mut s) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return Listener::Free;
    };
    let Some(secret) = state.and_then(|d| gitterrace_web::add_token(d, addr.port()).ok()) else {
        return Listener::Other;
    };
    let Ok(nonce) = gitterrace_actions::Token::generate() else {
        return Listener::Other;
    };
    if s.set_read_timeout(Some(PROBE_TIMEOUT)).is_err() {
        return Listener::Other;
    }
    let req = format!(
        "GET /_repos?nonce={} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n",
        nonce.as_str()
    );
    if s.write_all(req.as_bytes()).is_err() {
        return Listener::Other;
    }
    let mut buf = Vec::new();
    // A timeout after a partial read still leaves the bytes to judge.
    let _partial = s.take(64 * 1024).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let want = format!(
        "{{\"proof\":\"{}\"}}",
        gitterrace_actions::token_proof(&secret, nonce.as_str())
    );
    let body = text.split("\r\n\r\n").nth(1).unwrap_or_default();
    if text.starts_with("HTTP/1.1 200") && body.trim() == want {
        Listener::Gitterrace
    } else {
        Listener::Other
    }
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
mod tests;
