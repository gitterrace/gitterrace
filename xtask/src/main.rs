//! The generator behind `make docs-gen` and `make docs-check`: runs every `*_md` module and the
//! site writer, then writes the result or compares it with the tree.

#![expect(
    clippy::print_stdout,
    reason = "a developer command-line tool: the terminal is its output"
)]

mod action_scripts;
mod api_md;
mod changelog_md;
mod cli_md;
mod config_md;
mod docs_index;
mod docs_md;
mod facts;
mod openapi_json;
mod out;
mod plate;
mod readme_agents_md;
mod site;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
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
    /// Fail when a generated file is stale, writing nothing
    Check,
    /// Fail while a legal page still has a {{PLACEHOLDER}}; run before a deploy
    DeployCheck,
}

/// Every generator. A new one is checked by being listed here. `refresh` is false for
/// `check`, where a block made from the commits keeps its text (see `changelog_md`).
fn generators(root: &Path, refresh: bool) -> Result<Vec<Out>> {
    let ws = facts::load(root)?;
    let scripts = action_scripts::load(root)?;
    let mut outs = vec![
        api_md::render(),
        cli_md::render(),
        config_md::render(),
        docs_index::render(),
        openapi_json::render(&ws),
    ];
    outs.extend(readme_agents_md::render(&ws, &scripts));
    outs.extend(docs_md::render(root, &ws, &scripts)?);
    outs.extend(plate::outs(root, plate::PLATE_COLOR)?);
    outs.extend(changelog_md::render(root, refresh)?);
    outs.extend(site::render(root, &ws)?);
    Ok(outs)
}

fn main() -> Result<()> {
    let root = repo_root();
    match Cli::parse().cmd {
        Cmd::Generate => {
            // The site reads the documents from disk, so a block written in this pass shows
            // on its page in the next. Two passes settle it; a third change is a bug.
            for pass in 0..3 {
                let wrote = apply(&root, generators(&root, true)?, Mode::Write)?;
                for path in &wrote {
                    println!("  wrote {path}");
                }
                if wrote.is_empty() {
                    break;
                }
                anyhow::ensure!(pass < 2, "generate does not settle: {}", wrote.join(", "));
            }
            println!("generate: done");
        }
        Cmd::Check => {
            let outs = generators(&root, false)?;
            let orphans = unowned_blocks(&root, &outs)?;
            if !orphans.is_empty() {
                println!("generated blocks nobody fills: {}", orphans.join(", "));
                std::process::exit(1);
            }
            let ws = facts::load(&root)?;
            let repo = &ws
                .packages
                .get("gitterrace")
                .context("package gitterrace")?
                .repository;
            let site_bad = site::disk::check(&root, &outs, repo)?;
            let stale = apply(&root, outs, Mode::Check)?;
            if !stale.is_empty() {
                println!(
                    "generated output is stale, run `make docs-gen`: {}",
                    stale.join(", ")
                );
                std::process::exit(1);
            }
            println!("generated output: up to date");
            if !site_bad.is_empty() {
                println!(
                    "site/ out of step with the docs, or a page fails a site guard:\n  {}",
                    site_bad.join("\n  ")
                );
                std::process::exit(1);
            }
            println!(
                "site: every page is a published document in the docs order, with its title and \
                 description; every link and anchor lands, and no prose is on two pages"
            );
            let bad = docs_index::check(&root)?;
            if !bad.is_empty() {
                println!(
                    "documents out of step with the code:\n  {}",
                    bad.join("\n  ")
                );
                std::process::exit(1);
            }
            println!("documents: links, anchors, keys, flags and the index check out");
            site::legal::check(&root)?;
            println!("legal pages: the sources exist");
        }
        Cmd::DeployCheck => {
            let bad = site::legal::deploy_check(&root)?;
            if !bad.is_empty() {
                println!(
                    "the legal pages still have placeholders, fill them in before a deploy:\n  {}",
                    bad.join("\n  ")
                );
                std::process::exit(1);
            }
            println!("deploy-check: the legal pages have no placeholders");
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
