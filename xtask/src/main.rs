#![expect(
    clippy::print_stdout,
    reason = "a developer command-line tool: the terminal is its output"
)]

mod api_page;
mod config_page;
mod out;

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::out::{Mode, Out, apply};

#[derive(Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Regenerate every generated file
    Generate,
    /// Fail when a generated file is stale; writes nothing
    Check,
}

/// Every generator. A new one is checked by being listed here.
fn generators() -> Vec<Out> {
    vec![api_page::render(), config_page::render()]
}

fn main() -> Result<()> {
    let root = repo_root();
    match Cli::parse().cmd {
        Cmd::Generate => {
            for path in apply(&root, generators(), Mode::Write)? {
                println!("  wrote {path}");
            }
            println!("generate: done");
        }
        Cmd::Check => {
            let stale = apply(&root, generators(), Mode::Check)?;
            if !stale.is_empty() {
                println!(
                    "generated output is stale, run `make generate`: {}",
                    stale.join(", ")
                );
                std::process::exit(1);
            }
            println!("generated output: up to date");
        }
    }
    Ok(())
}

fn repo_root() -> PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    PathBuf::from(manifest_dir)
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}
