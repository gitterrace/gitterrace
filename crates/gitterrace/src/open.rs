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
/// Not configurable: a liveness probe of a server on this machine.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Why `open` stops before a browser opens. `Unknown` exits 2.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OpenError {
    #[error("{0}")]
    BadLink(#[from] gitterrace_actions::LinkError),
    #[error(
        "no local clone of {0} known. Run `git terrace` in it once, or list its folder in \
        links.roots"
    )]
    Unknown(String),
}

/// Why a request to the running server's `/_repos` route failed. The text is shown after the
/// command's own sentence.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ServerError {
    #[error("no state folder to read its secret from")]
    NoState,
    #[error("its secret is unreadable: {0}")]
    Secret(std::io::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("no answer")]
    NoAnswer,
    #[error("it predates adding repositories or does not allow it")]
    NoAdding,
    #[error("it predates removing repositories")]
    NoRemoving,
    #[error("HTTP {status}: {reason}")]
    Status { status: u16, reason: String },
}

/// The body of `POST /_repos`.
#[derive(serde::Serialize)]
struct AddBody<'a> {
    path: &'a Path,
}

/// The body of `DELETE /_repos`: a repository by top folder or by URL name.
#[derive(serde::Serialize)]
#[serde(untagged)]
enum RemoveBody<'a> {
    Path { path: PathBuf },
    Name { name: &'a str },
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
        .chain(found.iter().filter(|p| !roots.contains(p) || p.join(".git").exists()))
        .filter_map(|p| Repo::discover(p).ok())
        .filter(|r| r.id().as_deref() == Some(link.id.as_str()))
        .collect();
    let Some(first) = clones.first() else {
        return Err(OpenError::Unknown(link.id));
    };
    for r in &clones {
        let Some(full) = r.resolve(&link.sha).ok().filter(|f| f.starts_with(&link.sha)) else {
            continue;
        };
        let mut l = Link { sha: full.clone(), ..link };
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
        return Ok(Target { clone: r.root(), path: l.go(""), note });
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
    show(&target, print, loaded, config_dir)
}

#[expect(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// Starts or finds the server for `target.clone`, then opens or prints its page.
pub(super) fn show(
    target: &Target,
    print: bool,
    loaded: &gitterrace_config::Loaded,
    config_dir: Option<&Path>,
) -> anyhow::Result<()> {
    let cfg = &loaded.config;
    let state = gitterrace_web::state_dir();
    if let Some(n) = &target.note {
        eprintln!("gitterrace: {n}");
    }
    let configured: SocketAddr = cfg.server.addr.parse().context("server.addr")?;
    let addr = ensure_server(&target.clone, configured, state.as_deref(), &mut |c, a| {
        spawn_server(c, a, state.as_deref(), config_dir)
    })?;
    if let Some(hint) = std::env::current_exe().ok().and_then(|e| crate::handler::stale_hint(&e)) {
        eprintln!("gitterrace: {hint}");
    }
    let url = format!("http://{addr}{}", target.path);
    if print {
        println!("{url}");
        return Ok(());
    }
    browse(&url)
}

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
            anyhow::anyhow!("the server on {addr} could not add {}: {why}", root.display())
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
                        "a gitterrace server that does not serve this repository and could not add \
                        it ({why})"
                    ),
                }
            } else {
                "another program".to_owned()
            };
            let l = TcpListener::bind((addr.ip(), 0)).context("finding a free port")?;
            let other = l.local_addr()?;
            // The listener only found a free port; the child binds it again on its own. Another
            // process could take it in between, on loopback, and the child then fails to start.
            drop(l);
            eprintln!(
                "gitterrace: {addr} is held by {what}; it is left running and this repository gets \
                its own server at {other}. Restart the server on {addr} with this repository added \
                to see it there."
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

/// Asks the gitterrace server on `addr` to add the repository at `root` (canonical). Accepted
/// (202, it opens in the background) and already served (409) both count as added.
/// The `/r/NAME/` prefix the server serves `root` under, new or already there.
fn add_repo(addr: SocketAddr, state: Option<&Path>, root: &Path) -> Result<String, ServerError> {
    match secret_request(addr, state, "POST", &AddBody { path: root })? {
        (202 | 409, v) => Ok(v.base.unwrap_or_else(|| "/".to_owned())),
        (404 | 405, _) => Err(ServerError::NoAdding),
        (status, v) => Err(ServerError::Status { status, reason: v.why() }),
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
        RemoveBody::Path { path: root.canonicalize().unwrap_or(root) }
    } else {
        RemoveBody::Name { name: target }
    };
    match identify(addr, state) {
        Listener::Gitterrace => {}
        Listener::Free => anyhow::bail!("no gitterrace server is running on {addr}"),
        Listener::Other => anyhow::bail!(
            "{addr} is held by a program that is not a gitterrace server of this user, or one \
            started with --public"
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
    body: &RemoveBody<'_>,
) -> Result<Removed, ServerError> {
    match secret_request(addr, state, "DELETE", body)? {
        (200, v) => Ok(Removed::Done(v.name.unwrap_or_default())),
        (404, v) if v.error.is_some() => Ok(Removed::NotServed),
        (404 | 405, _) => Err(ServerError::NoRemoving),
        (status, v) => Err(ServerError::Status { status, reason: v.why() }),
    }
}

/// What `/_repos` answers, the fields the CLI reads. Absent keys and a body that is not JSON
/// leave them `None`; `text` keeps the body for the message when nothing else explains it.
#[derive(Debug, Default, serde::Deserialize)]
struct RepoReply {
    base: Option<String>,
    error: Option<String>,
    name: Option<String>,
    #[serde(skip)]
    text: String,
}

impl RepoReply {
    fn parse(body: &str) -> Self {
        let mut r: Self = serde_json::from_str(body).unwrap_or_default();
        body.clone_into(&mut r.text);
        r
    }

    /// The `error` of a JSON reply, or the start of the reply itself.
    fn why(&self) -> String {
        self.error.clone().unwrap_or_else(|| self.text.chars().take(200).collect())
    }
}

/// One `/_repos` request carrying the secret from the 0600 file the server on `addr` wrote to
/// `state`, so only its own user can ask. The status and the reply body.
fn secret_request(
    addr: SocketAddr,
    state: Option<&Path>,
    method: &str,
    body: &impl serde::Serialize,
) -> Result<(u16, RepoReply), ServerError> {
    let state = state.ok_or(ServerError::NoState)?;
    let token = gitterrace_web::add_token(state, addr.port()).map_err(ServerError::Secret)?;
    let body = serde_json::to_string(body).map_err(std::io::Error::from)?;
    let s = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT)?;
    let req = format!(
        "{method} /_repos HTTP/1.1\r\nHost: {addr}\r\n{}: {token}\r\nContent-Type: \
        application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        gitterrace_web::ADD_TOKEN_HEADER,
        body.len()
    );
    // Building the repository's router opens its index; give it longer than a probe.
    let text = exchange(s, &req, READY_WAIT)?;
    let (Some(status), reply) = split_response(&text) else {
        return Err(ServerError::NoAnswer);
    };
    Ok((status, RepoReply::parse(reply.trim())))
}

/// Sends `request` on `s` and reads the whole answer, at most 64 KiB. A timeout after a partial
/// read still leaves what arrived to judge.
fn exchange(mut s: TcpStream, request: &str, timeout: Duration) -> std::io::Result<String> {
    s.set_read_timeout(Some(timeout))?;
    s.write_all(request.as_bytes())?;
    let mut buf = Vec::new();
    let _partial = s.take(64 * 1024).read_to_end(&mut buf);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The status code of an HTTP/1.1 answer, when it has one, and its body.
fn split_response(text: &str) -> (Option<u16>, &str) {
    let status = text.get(9..12).and_then(|c| c.parse::<u16>().ok());
    (status, text.split("\r\n\r\n").nth(1).unwrap_or_default())
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
    cmd.arg("serve").stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
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
    let Ok(s) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return Listener::Free;
    };
    let Some(secret) = state.and_then(|d| gitterrace_web::add_token(d, addr.port()).ok()) else {
        return Listener::Other;
    };
    let Ok(nonce) = gitterrace_actions::Token::generate() else {
        return Listener::Other;
    };
    let req = format!(
        "GET /_repos?nonce={} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n",
        nonce.as_str()
    );
    let Ok(text) = exchange(s, &req, PROBE_TIMEOUT) else {
        return Listener::Other;
    };
    let want =
        format!("{{\"proof\":\"{}\"}}", gitterrace_actions::token_proof(&secret, nonce.as_str()));
    let (status, body) = split_response(&text);
    if status == Some(200) && body.trim() == want { Listener::Gitterrace } else { Listener::Other }
}

/// Whether `addr` answers `GET path` with a redirect: `/go` redirects only for a repository
/// it serves, so a server for another repository (or not gitterrace at all) says no.
fn serves(addr: SocketAddr, path: &str) -> bool {
    let Ok(s) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return false;
    };
    let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    exchange(s, &req, PROBE_TIMEOUT).is_ok_and(|text| split_response(&text).0 == Some(302))
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
    gitterrace_config::home_dir()
        .map_or_else(|| PathBuf::from(p), |home| gitterrace_config::expand_tilde(p, &home))
}

/// The platform's URL opener. The URL is built here from validated parts and starts with
/// `http://`, so it can never read as an option.
fn browse(url: &str) -> anyhow::Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if crate::handler::is_wsl() {
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

pub(crate) use range::run_range;
mod range;

#[cfg(test)]
mod tests;
