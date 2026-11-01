//! The generator behind `make docs-gen` and `make docs-check`: runs every `*_md` module and the
//! site writer, then writes the result or compares it with the tree.

#![expect(
    clippy::print_stdout,
    reason = "a developer command-line tool: the terminal is its output"
)]

mod action_scripts;
mod api_md;
mod config_md;
mod docs_md;
mod facts;
mod out;
mod readme_agents_md;
mod site;

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::out::{Mode, Out, apply, unowned_blocks};

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
fn generators(root: &Path) -> Result<Vec<Out>> {
    let ws = facts::load(root)?;
    let scripts = action_scripts::load(root)?;
    let mut outs = vec![api_md::render(), config_md::render()];
    outs.extend(readme_agents_md::render(&ws, &scripts));
    outs.extend(docs_md::render(root, &ws, &scripts)?);
    outs.extend(site::render(root, &ws)?);
    Ok(outs)
}

fn main() -> Result<()> {
    let root = repo_root();
    match Cli::parse().cmd {
        Cmd::Generate => {
            for path in apply(&root, generators(&root)?, Mode::Write)? {
                println!("  wrote {path}");
            }
            println!("generate: done");
        }
        Cmd::Check => {
            let outs = generators(&root)?;
            let orphans = unowned_blocks(&root, &outs)?;
            if !orphans.is_empty() {
                println!("generated blocks nobody fills: {}", orphans.join(", "));
                std::process::exit(1);
            }
            let stale = apply(&root, outs, Mode::Check)?;
            if !stale.is_empty() {
                println!(
                    "generated output is stale, run `make docs-gen`: {}",
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
