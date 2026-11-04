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
mod open;

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
    let cli = Cli::parse();
    let path = cli
        .paths
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    // stdout carries the MCP protocol, so logs always go to stderr.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let dir = gitterrace_config::config_dir(cli.config.as_deref());
    let flags = Flags {
        dir: dir.clone(),
        addr: cli.addr,
        poll: cli.poll,
        no_watch: cli.no_watch,
        public: cli.public,
    };
    let loaded = flags.load().context("reading config.toml")?;
    for w in &loaded.warnings {
        tracing::warn!("config: {w}");
    }
    match cli.command.unwrap_or(Command::Serve) {
        Command::Config => print!("{}", loaded.describe()),
        Command::Serve => {
            let addr: SocketAddr = loaded.config.server.addr.parse().context("server.addr")?;
            let actions = dir
                .clone()
                .filter(|_| !cli.no_actions)
                .map(|d| d.join("actions.toml"));
            let m = &loaded.config.multi;
            let repos = gitterrace_web::discover_repos(&cli.paths, m.depth, &m.skip_dirs);
            if !loaded.config.server.public
                && open::join_running(addr, &repos, gitterrace_web::state_dir().as_deref())?
            {
                return Ok(());
            }
            let watched = dir.as_ref().map(|d| gitterrace_web::Watched {
                file: d.join("config.toml"),
                load: Box::new(move || flags.load().map(|l| l.config)),
            });
            gitterrace_web::serve(repos, addr, actions, loaded.config, watched).await?;
        }
        Command::Mcp => {
            gitterrace_api::serve_mcp(&path, &loaded.config).await?;
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
        Command::Remove { target } => {
            let addr: SocketAddr = loaded.config.server.addr.parse().context("server.addr")?;
            tokio::task::block_in_place(|| open::remove_cmd(addr, target.as_deref()))?;
        }
        Command::Init => init(cli.config.as_deref())?,
        Command::InstallHandler { uninstall } => handler::run(uninstall)?,
        Command::Wrapped {
            year,
            layout,
            theme,
            out,
        } => wrapped(&path, year, &layout, &theme, out)?,
        Command::Export {
            out,
            force,
            commits,
            max_file_kb,
            max_pages,
            project,
        } => {
            let scope = export::Scope {
                commits,
                max_file: max_file_kb * 1024,
                force,
                projects: project
                    .iter()
                    .map(|p| export::Project::parse(p))
                    .collect::<anyhow::Result<_>>()?,
            };
            let pages = export::budget(max_pages, &loaded.config);
            export_cmd(&cli.paths, &path, out, &scope, pages, &loaded.config).await?;
        }
    }
    Ok(())
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
        (
            "actions.toml",
            gitterrace_actions::Config::STARTER.to_owned(),
        ),
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
    poll: Option<u64>,
    no_watch: bool,
    public: bool,
}

impl Flags {
    /// The file with the flags laid over it, validated.
    fn load(&self) -> Result<gitterrace_config::Loaded, gitterrace_config::ConfigError> {
        let mut loaded = gitterrace_config::load(self.dir.as_deref())?;
        if let Some(a) = self.addr {
            loaded.config.server.addr = a.to_string();
            loaded.flagged("server.addr");
        }
        if let Some(ms) = self.poll {
            loaded.config.watch.interval_ms = ms;
            loaded.flagged("watch.interval_ms");
        }
        if self.no_watch {
            loaded.config.watch.enabled = false;
            loaded.flagged("watch.enabled");
        }
        if self.public {
            loaded.config.server.public = true;
            loaded.flagged("server.public");
        }
        loaded.validate()?;
        Ok(loaded)
    }
}

#[expect(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "user-facing CLI output: this is the command's result or diagnostic, not a log line"
)]
fn wrapped(
    path: &std::path::Path,
    year: Option<i32>,
    layout: &str,
    theme: &str,
    out: Option<PathBuf>,
) -> anyhow::Result<()> {
    let repo = gitterrace_git::Repo::discover(path).context("opening the repository")?;
    let index = gitterrace_index::Index::open(&repo).context("opening the cache")?;
    eprintln!("updating the cache (first run reads every commit once)...");
    index
        .update(&repo, "HEAD", &|_| {})
        .context("updating the cache")?;
    let records = index.records(&repo, "HEAD").context("reading the cache")?;
    let year = year.unwrap_or_else(|| {
        let newest = records
            .iter()
            .map(gitterrace_index::local_day)
            .max()
            .unwrap_or(0);
        i32::try_from(gitterrace_git::civil_from_days(newest).0).unwrap_or(i32::MAX)
    });
    let stats = gitterrace_cards::wrapped_stats(&repo.name(), &records, year);
    let layout = gitterrace_cards::Layout::parse(layout).context("unknown layout")?;
    let theme = gitterrace_cards::Theme::parse(theme).context("unknown theme")?;
    let svg = gitterrace_cards::wrapped_svg(&stats, layout, theme);
    let out = out.unwrap_or_else(|| PathBuf::from(format!("wrapped-{year}.svg")));
    std::fs::write(&out, svg).with_context(|| format!("writing {}", out.display()))?;
    println!("{}", out.display());
    Ok(())
}

/// Exports the repositories `serve` would show for `paths`, so an export of a folder of repos is
/// the multi-repo site, index and all.
#[expect(clippy::print_stdout, reason = "the command's result line")]
async fn export_cmd(
    paths: &[PathBuf],
    path: &Path,
    out: Option<PathBuf>,
    scope: &export::Scope,
    pages: usize,
    config: &gitterrace_config::Config,
) -> anyhow::Result<()> {
    let out = out.unwrap_or_else(|| PathBuf::from(export::DEFAULT_OUT));
    let m = &config.multi;
    let repos = gitterrace_web::discover_repos(paths, m.depth, &m.skip_dirs);
    if repos.is_empty() {
        anyhow::bail!("no git repository found in {}", path.display());
    }
    let n = export::export(&repos, &out, scope, pages).await?;
    println!("wrote {n} files to {}", out.display());
    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use crate::init_in;

    fn files() -> [(&'static str, String); 2] {
        [
            ("config.toml", gitterrace_config::template()),
            (
                "actions.toml",
                gitterrace_actions::Config::STARTER.to_owned(),
            ),
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
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("actions.toml")).expect("read"),
            "mine"
        );
        assert_eq!(mode(&tmp.path().join("config.toml")), 0o600);
    }
}
