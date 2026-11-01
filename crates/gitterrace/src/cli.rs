//! The command line: flags and subcommands. Kept free of crate items so `xtask` can include it.

use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

/// Browse a git repository in your browser, offline.
#[derive(Parser)]
#[command(version = option_env!("GITTERRACE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))]
pub(crate) struct Cli {
    /// Repositories to serve (any path inside one), or folders to search two levels deep.
    /// Other commands use the first.
    #[arg(default_value = ".", global = true, num_args = 1..)]
    pub(crate) paths: Vec<PathBuf>,
    /// Address to listen on. Local only by default. Overrides `server.addr`.
    #[arg(long, global = true)]
    pub(crate) addr: Option<SocketAddr>,
    /// Check for repository changes every N milliseconds (at least 250). Overrides
    /// `watch.interval_ms`.
    #[arg(long, global = true, value_name = "MS")]
    pub(crate) poll: Option<u64>,
    /// Do not watch for changes: no live updates, and a restart or reload picks up new commits.
    /// Overrides `watch.enabled`.
    #[arg(long, global = true)]
    pub(crate) no_watch: bool,
    /// Folder holding `config.toml` (and `actions.toml`). Defaults to the XDG config folder.
    #[arg(long, global = true)]
    pub(crate) config: Option<PathBuf>,
    /// Never run user actions, even when `actions.toml` declares some.
    #[arg(long, global = true)]
    pub(crate) no_actions: bool,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Serve the repository in the browser (the default).
    Serve,
    /// Print the effective configuration and where each value comes from.
    Config,
    /// Run a read-only MCP server over stdio for agents.
    Mcp,
    /// Write the year-in-review card for the repository.
    Wrapped {
        /// Year to summarize. Defaults to the year of the newest commit.
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
    /// Open a `gitterrace://` or `/go` permalink in the browser, starting a server if needed.
    ///
    /// The clone is found among repositories served before and under `links.roots`. An
    /// unknown repository exits 2, with no clone and no network.
    Open {
        /// `gitterrace://<id>/<sha>[/<path>][#L<a>-L<b>]`, an `http(s)://.../go/...` URL or a
        /// bare `/go/...` path.
        link: String,
        /// Print the URL instead of opening a browser.
        #[arg(long)]
        print: bool,
    },
    /// Register (or with `--uninstall` remove) this program as the `gitterrace://` link handler.
    ///
    /// Opt-in. macOS writes `~/Applications/gitterrace-open.app`, Linux a `.desktop` file, and
    /// WSL only prints the steps. It prints exactly what it wrote.
    InstallHandler {
        /// Remove what a previous install wrote.
        #[arg(long)]
        uninstall: bool,
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
