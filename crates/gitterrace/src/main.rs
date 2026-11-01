use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use clap::Parser;

mod cli;
mod export;

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
            gitterrace_api::serve_mcp(&path).await?;
        }
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
        } => {
            let scope = export::Scope {
                commits,
                max_file: max_file_kb * 1024,
            };
            let n = export::export(&path, &out, &scope).await?;
            println!("wrote {n} files to {}", out.display());
        }
    }
    Ok(())
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

/// Civil year of a Unix timestamp (UTC), without a date dependency.
fn year_of(secs: i64) -> i32 {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let year = yoe + era * 400 + i64::from(mp >= 10);
    i32::try_from(year).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use crate::year_of;

    #[test]
    fn year_of_known_dates() {
        assert_eq!(year_of(0), 1970);
        assert_eq!(year_of(1_798_761_599), 2026); // 2026-12-31T23:59:59Z
        assert_eq!(year_of(1_798_761_600), 2027);
        assert_eq!(year_of(951_782_400), 2000); // 2000-02-29
    }
}
