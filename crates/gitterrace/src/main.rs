use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};

/// A GitHub/GitLab-style view of a git repository, offline.
#[derive(Parser)]
#[command(version = option_env!("GITTERRACE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))]
struct Cli {
    /// Repository to serve (any path inside it).
    #[arg(default_value = ".", global = true)]
    path: PathBuf,
    /// Address to listen on. Local only by default.
    #[arg(long, default_value = "127.0.0.1:4242")]
    addr: SocketAddr,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the repository in the browser (the default).
    Serve,
    /// Run a read-only MCP server over stdio for agents.
    Mcp,
    /// Write the year-in-review card for the repository.
    Wrapped {
        /// Year to summarise. Defaults to the year of the newest commit.
        #[arg(long)]
        year: Option<i32>,
        #[arg(long, value_enum, default_value_t = Shape::Landscape)]
        layout: Shape,
        #[arg(long, value_enum, default_value_t = Look::Dark)]
        theme: Look,
        /// Write a PNG instead of an SVG (needs the `png` build feature).
        #[arg(long)]
        png: bool,
        /// Output file. Defaults to wrapped-<year>.svg or .png.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Shape {
    Landscape,
    Portrait,
}

#[derive(Clone, Copy, ValueEnum)]
enum Look {
    Light,
    Dark,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => {
            tracing_subscriber::fmt().init();
            gitterrace_web::serve(cli.path, cli.addr).await?;
        }
        // stdout carries the MCP protocol, so logs go to stderr.
        Command::Mcp => {
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .init();
            gitterrace_api::serve_mcp(&cli.path).await?;
        }
        Command::Wrapped {
            year,
            layout,
            theme,
            png,
            out,
        } => wrapped(&cli.path, year, layout, theme, png, out)?,
    }
    Ok(())
}

fn wrapped(
    path: &std::path::Path,
    year: Option<i32>,
    layout: Shape,
    theme: Look,
    png: bool,
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
    let layout = match layout {
        Shape::Landscape => gitterrace_cards::Layout::Landscape,
        Shape::Portrait => gitterrace_cards::Layout::Portrait,
    };
    let theme = match theme {
        Look::Light => gitterrace_cards::Theme::Light,
        Look::Dark => gitterrace_cards::Theme::Dark,
    };
    let svg = gitterrace_cards::wrapped_svg(&stats, layout, theme);
    let (bytes, ext) = if png {
        (rasterize(&svg)?, "png")
    } else {
        (svg.into_bytes(), "svg")
    };
    let out = out.unwrap_or_else(|| PathBuf::from(format!("wrapped-{year}.{ext}")));
    std::fs::write(&out, bytes).with_context(|| format!("writing {}", out.display()))?;
    println!("{}", out.display());
    Ok(())
}

#[cfg(feature = "png")]
fn rasterize(svg: &str) -> anyhow::Result<Vec<u8>> {
    Ok(gitterrace_cards::png::render_scaled(svg, 2.0)?)
}

#[cfg(not(feature = "png"))]
fn rasterize(_svg: &str) -> anyhow::Result<Vec<u8>> {
    anyhow::bail!("this build has no PNG support; rebuild with `--features png` or use the SVG")
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
