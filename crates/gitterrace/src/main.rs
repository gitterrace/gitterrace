use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use clap::Parser;

mod cli;
mod export;
mod handler;
mod open;

use cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
            let watched = dir.as_ref().map(|d| gitterrace_web::Watched {
                file: d.join("config.toml"),
                load: Box::new(move || flags.load().map(|l| l.config)),
            });
            gitterrace_web::serve(repos, addr, actions, loaded.config, watched).await?;
        }
        Command::Mcp => {
            let style = gitterrace_actions::LinkStyle::from_config(&loaded.config.links.style);
            gitterrace_api::serve_mcp(&path, style).await?;
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
            commits,
            max_file_kb,
            project,
        } => {
            let scope = export::Scope {
                commits,
                max_file: max_file_kb * 1024,
                projects: project
                    .iter()
                    .map(|p| export::Project::parse(p))
                    .collect::<anyhow::Result<_>>()?,
            };
            // The same repositories `serve` would show for these paths, so an export of a
            // folder of repos is the multi-repo site, index and all.
            let m = &loaded.config.multi;
            let repos = gitterrace_web::discover_repos(&cli.paths, m.depth, &m.skip_dirs);
            if repos.is_empty() {
                anyhow::bail!("no git repository found in {}", path.display());
            }
            let n = export::export(&repos, &out, &scope).await?;
            println!("wrote {n} files to {}", out.display());
        }
    }
    Ok(())
}

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
        println!("{} {state}", path.display());
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
        loaded.validate()?;
        Ok(loaded)
    }
}

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
    let newest = records.iter().map(|r| r.time).max().unwrap_or(0);
    let year = year.unwrap_or_else(|| year_of(newest));
    let stats = gitterrace_cards::wrapped_stats(&repo.name(), &records, year);
    let layout = gitterrace_cards::Layout::parse(layout).context("unknown layout")?;
    let theme = gitterrace_cards::Theme::parse(theme).context("unknown theme")?;
    let svg = gitterrace_cards::wrapped_svg(&stats, layout, theme);
    let out = out.unwrap_or_else(|| PathBuf::from(format!("wrapped-{year}.svg")));
    std::fs::write(&out, svg).with_context(|| format!("writing {}", out.display()))?;
    println!("{}", out.display());
    Ok(())
}

/// Civil year of a Unix timestamp (UTC).
fn year_of(secs: i64) -> i32 {
    let (year, _, _) = gitterrace_git::civil_from_days(secs.div_euclid(86_400));
    i32::try_from(year).unwrap_or(i32::MAX)
}

#[cfg(test)]
#[cfg(unix)]
#[allow(clippy::expect_used)]
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
