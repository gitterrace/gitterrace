use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand};

mod export;

/// A GitHub/GitLab-style view of a git repository, offline.
#[derive(Parser)]
#[command(version = option_env!("GITTERRACE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))]
struct Cli {
    /// Repositories to serve (any path inside one), or folders to search two levels deep.
    /// Other commands use the first.
    #[arg(default_value = ".", global = true, num_args = 1..)]
    paths: Vec<PathBuf>,
    /// Address to listen on. Local only by default. Overrides `server.addr`.
    #[arg(long, global = true)]
    addr: Option<SocketAddr>,
    /// Folder holding `config.toml` (and `actions.toml`). Defaults to the XDG config folder.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Never run user actions, even when `actions.toml` declares some.
    #[arg(long, global = true)]
    no_actions: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the repository in the browser (the default).
    Serve,
    /// Print the effective configuration and where each value comes from.
    Config,
    /// Run a read-only MCP server over stdio for agents.
    Mcp,
    /// Write the year-in-review card for the repository.
    Wrapped {
        /// Year to summarise. Defaults to the year of the newest commit.
        #[arg(long)]
        year: Option<i32>,
        #[arg(long, value_parser = ["landscape", "portrait"], default_value = "landscape")]
        layout: String,
        #[arg(long, value_parser = ["light", "dark"], default_value = "dark")]
        theme: String,
        /// Output file. Defaults to `wrapped-YEAR.svg`.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Write a static copy of the site to a folder, for sharing or hosting.
    Export {
        /// Folder to write into (created if missing).
        out: PathBuf,
        /// Newest commits to include as pages.
        #[arg(long, default_value_t = 500)]
        commits: usize,
        /// Largest file to include as a page, in KiB.
        #[arg(long, default_value_t = 512)]
        max_file_kb: u64,
    },
}

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
    let mut loaded = gitterrace_config::load(dir.as_deref()).context("reading config.toml")?;
    for w in &loaded.warnings {
        tracing::warn!("config: {w}");
    }
    if let Some(a) = cli.addr {
        loaded.config.server.addr = a.to_string();
        loaded.flagged("server.addr");
    }
    loaded.validate().context("config")?;
    match cli.command.unwrap_or(Command::Serve) {
        Command::Config => print!("{}", loaded.describe()),
        Command::Serve => {
            let addr: SocketAddr = loaded.config.server.addr.parse().context("server.addr")?;
            let actions = dir
                .filter(|_| !cli.no_actions)
                .map(|d| d.join("actions.toml"));
            gitterrace_web::serve(gitterrace_web::discover_repos(&cli.paths), addr, actions)
                .await?;
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
