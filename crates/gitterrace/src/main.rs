use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::Context;
use clap::Parser;

#[cfg(not(windows))]
mod alloc;
mod cli;
mod export;
mod handler;
mod insights;
mod open;
mod service;

use cli::{Cli, Command};

// See Cargo.toml: the system allocator kept an idle server's freed record sets resident.
#[cfg(not(windows))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[expect(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(not(windows))]
    if alloc::start()? {
        return Ok(());
    }
    // An index child runs its update and exits here, before any argument is parsed.
    gitterrace_web::index_child();
    let cli = Cli::parse();
    let path = cli.paths.first().cloned().unwrap_or_else(|| PathBuf::from("."));
    // stdout carries the MCP protocol, so logs always go to stderr.
    // Color only for a person at a terminal: NO_COLOR (any non-empty value) and a piped stderr
    // get plain lines. Export progress redraws one line only on a terminal, with no escape codes.
    tracing_subscriber::fmt().with_writer(std::io::stderr).with_ansi(use_color()).init();
    let dir = gitterrace_config::config_dir(cli.config.as_deref());
    let flags =
        Flags { dir: dir.clone(), addr: cli.addr, no_watch: cli.no_watch, public: cli.public };
    let loaded = flags.load().context("reading config.toml")?;
    for w in &loaded.warnings {
        tracing::warn!("config: {w}");
    }
    if let Some(spec) = &cli.range {
        return tokio::task::block_in_place(|| {
            open::run_range(spec, &path, cli.print, &loaded, cli.config.as_deref())
        });
    }
    match cli.command.unwrap_or(Command::Serve) {
        Command::Config => print!("{}", loaded.describe()),
        Command::Serve => {
            let addr = server_addr(&loaded.config)?;
            let actions = dir.clone().filter(|_| !cli.no_actions).map(|d| d.join("actions.toml"));
            let repos = repos_of(&cli.paths, &loaded.config);
            if !loaded.config.server.is_public
                && open::join_running(addr, &repos, gitterrace_web::state_dir().as_deref())?
            {
                return Ok(());
            }
            let watched = dir.as_ref().map(|d| gitterrace_web::Watched {
                file: d.join("config.toml"),
                load: Box::new(move || flags.load().map(|l| l.config)),
            });
            gitterrace_web::serve(repos, addr, actions, loaded.config, watched, &cli.paths).await?;
        }
        Command::Mcp { served } => {
            let repos = repos_of(&cli.paths, &loaded.config);
            if served || repos.len() > 1 {
                let (scope, list) = gitterrace_web::mcp_scope(&repos, served);
                gitterrace_api::serve_mcp_repos(&scope, list, &loaded.config).await?;
            } else {
                gitterrace_api::serve_mcp(&path, &loaded.config).await?;
            }
        }
        Command::Insights { name, rev, period, q, label, metric, limit, coupling, json } => {
            let params = gitterrace_api::Params::new(period, q, label, metric, limit);
            let params = coupling.apply(params);
            let out = tokio::task::block_in_place(|| {
                insights::run(&path, &loaded.config, name.as_deref(), rev.as_deref(), &params, json)
            })?;
            println!("{out}");
        }
        Command::Open { link, print } => {
            let r = tokio::task::block_in_place(|| {
                open::run(&link, print, &loaded, cli.config.as_deref())
            });
            if let Err(e) = r {
                if let Some(o) = e.downcast_ref::<open::OpenError>() {
                    eprintln!("gitterrace: {o}");
                    std::process::exit(2);
                }
                return Err(e);
            }
        }
        Command::Launch { print } => {
            let addr = server_addr(&loaded.config)?;
            tokio::task::block_in_place(|| open::launch_cmd(addr, print))?;
        }
        Command::Remove { target } => {
            let addr = server_addr(&loaded.config)?;
            tokio::task::block_in_place(|| open::remove_cmd(addr, target.as_deref()))?;
        }
        Command::Service { action, now } => service::run(action, now, &cli.paths, cli.addr)?,
        Command::Init => init(cli.config.as_deref())?,
        Command::InstallHandler { uninstall } => handler::run(uninstall)?,
        Command::Export { out, force, commits, file_kb, export_pages, quiet, project } => {
            let scope = export::Scope {
                commits,
                max_file: file_kb * 1024,
                force,
                projects: project
                    .iter()
                    .map(|p| export::Project::parse(p))
                    .collect::<anyhow::Result<_>>()?,
                config: loaded.config.clone(),
            };
            let pages = export::budget(export_pages, &loaded.config);
            let repos = repos_of(&cli.paths, &loaded.config);
            if repos.is_empty() {
                anyhow::bail!("no git repository found in {}", path.display());
            }
            let out = out.unwrap_or_else(|| PathBuf::from(export::DEFAULT_OUT));
            export_cmd(&repos, &out, &scope, pages, quiet).await?;
        }
    }
    Ok(())
}

/// The address `server.addr` names.
fn server_addr(config: &gitterrace_config::Config) -> anyhow::Result<SocketAddr> {
    config.server.addr.parse().context("server.addr")
}

/// The repositories `serve` shows for `paths`, found as `[multi]` says.
fn repos_of(paths: &[PathBuf], config: &gitterrace_config::Config) -> Vec<PathBuf> {
    let m = &config.multi;
    gitterrace_web::discover_repos(paths, m.depth, &m.skip_dirs)
}

#[expect(
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
/// Writes `config.toml` and `actions.toml` into the config folder, private to the user, never
/// over an existing file. Prints one line per file.
fn init(config: Option<&std::path::Path>) -> anyhow::Result<()> {
    let dir =
        gitterrace_config::config_dir(config).context("no config folder: set --config or HOME")?;
    let files = [
        ("config.toml", gitterrace_config::template()),
        ("actions.toml", gitterrace_actions::Config::STARTER.to_owned()),
    ];
    for (path, state) in init_in(&dir, &files)? {
        println!("{}: {state}", path.display());
    }
    Ok(())
}

/// The write step of `init`: `(path, "written" | "kept")` per file.
fn init_in(
    dir: &std::path::Path,
    files: &[(&str, String)],
) -> anyhow::Result<Vec<(PathBuf, &'static str)>> {
    use std::io::Write as _;
    #[cfg(unix)]
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};

    let mut dirs = std::fs::DirBuilder::new();
    dirs.recursive(true);
    // Windows has no mode bits: the folder lives under `%APPDATA%`, private to the user.
    #[cfg(unix)]
    dirs.mode(0o700);
    dirs.create(dir).context("creating the config folder")?;
    let mut done = Vec::new();
    for (name, text) in files {
        let path = dir.join(name);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let opened = opts.open(&path);
        match opened {
            Ok(mut f) => {
                f.write_all(text.as_bytes())?;
                done.push((path, "written"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => done.push((path, "kept")),
            Err(e) => return Err(e).with_context(|| format!("writing {}", path.display())),
        }
    }
    Ok(done)
}

/// The command-line flags that beat `config.toml`, kept to load the file again on reload.
#[derive(Clone)]
struct Flags {
    dir: Option<PathBuf>,
    addr: Option<SocketAddr>,
    no_watch: bool,
    public: bool,
}

impl Flags {
    fn load(&self) -> Result<gitterrace_config::Loaded, gitterrace_config::ConfigError> {
        let mut loaded = gitterrace_config::load(self.dir.as_deref())?;
        if let Some(a) = self.addr {
            loaded.config.server.addr = a.to_string();
            loaded.flagged("server.addr");
        }
        if self.no_watch {
            loaded.config.watch.enabled = false;
            loaded.flagged("watch.enabled");
        }
        if self.public {
            loaded.config.server.is_public = true;
            loaded.flagged("server.is_public");
        }
        loaded.config.validate()?;
        Ok(loaded)
    }
}

/// Exports the repositories `serve` would show for `paths`, so an export of a folder of repos is
/// the multi-repo site, index and all.
#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the command's result line and its hosting note"
)]
async fn export_cmd(
    repos: &[PathBuf],
    out: &Path,
    scope: &export::Scope,
    pages: Option<usize>,
    quiet: bool,
) -> anyhow::Result<()> {
    let mut progress =
        export::Progress::new(export::Mode::detect(quiet), Box::new(std::io::stderr()));
    let n = export::export_with(repos, out, scope, pages, &mut progress).await?;
    println!("wrote {n} files to {}", out.display());
    if !quiet {
        eprintln!(
            "note: _headers carries the CSP; Netlify and Cloudflare Pages apply it, other hosts \
             need the same headers set by hand (docs/online.md)"
        );
    }
    Ok(())
}

/// Whether log lines may carry color codes: stderr is a terminal and `NO_COLOR` is unset or empty.
fn use_color() -> bool {
    use std::io::IsTerminal as _;
    std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()) && std::io::stderr().is_terminal()
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use crate::init_in;

    fn files() -> [(&'static str, String); 2] {
        [
            ("config.toml", gitterrace_config::template()),
            ("actions.toml", gitterrace_actions::Config::STARTER.to_owned()),
        ]
    }

    fn mode(p: &std::path::Path) -> u32 {
        std::fs::metadata(p).expect("meta").permissions().mode() & 0o777
    }

    #[test]
    fn init_writes_both_private_into_an_empty_folder() {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path().join("gt");
        let done = init_in(&dir, &files()).expect("init");
        assert!(done.iter().all(|(_, s)| *s == "written"));
        assert_eq!(mode(&dir), 0o700);
        for n in ["config.toml", "actions.toml"] {
            assert_eq!(mode(&dir.join(n)), 0o600);
        }
    }

    #[test]
    fn init_keeps_existing_files_byte_identical() {
        let tmp = tempfile::tempdir().expect("tmp");
        for n in ["config.toml", "actions.toml"] {
            std::fs::write(tmp.path().join(n), format!("mine {n}")).expect("write");
        }
        let done = init_in(tmp.path(), &files()).expect("init");
        assert!(done.iter().all(|(_, s)| *s == "kept"));
        for n in ["config.toml", "actions.toml"] {
            assert_eq!(
                std::fs::read_to_string(tmp.path().join(n)).expect("read"),
                format!("mine {n}")
            );
        }
    }

    #[test]
    fn init_writes_only_the_missing_config() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("actions.toml"), "mine").expect("write");
        let done = init_in(tmp.path(), &files()).expect("init");
        assert_eq!(done[0].1, "written");
        assert_eq!(done[1].1, "kept");
        assert_eq!(std::fs::read_to_string(tmp.path().join("actions.toml")).expect("read"), "mine");
        assert_eq!(mode(&tmp.path().join("config.toml")), 0o600);
    }
}
