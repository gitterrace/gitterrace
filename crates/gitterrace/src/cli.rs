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
    /// Retired: changes come from OS file events. Accepted with a warning so old scripts start.
    #[arg(long, global = true, value_name = "MS", hide = true)]
    pub(crate) interval_ms: Option<u64>,
    /// Do not watch for changes: no live updates, and a restart or reload picks up new commits.
    /// Overrides `watch.enabled`.
    #[arg(long, global = true)]
    pub(crate) no_watch: bool,
    /// Folder holding `config.toml` (and `actions.toml`). Default: the XDG config folder.
    #[arg(long, global = true, value_name = "DIR")]
    pub(crate) config: Option<PathBuf>,
    /// Never run user actions, even when `actions.toml` declares some.
    #[arg(long, global = true)]
    pub(crate) no_actions: bool,
    /// Serve read-only to other machines: committed history only. Required with any `--addr`
    /// that is not loopback. Overrides `server.is_public`.
    #[arg(long, global = true)]
    pub(crate) public: bool,
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
    ///
    /// Reads the repositories the paths name (folders are searched as for `serve`). With
    /// `--served` it also reads every repository served before, such as those of a running
    /// server. Tools then take a `repo` name from `list_repos`.
    Mcp {
        /// Also read the repositories served before (the registry a running server keeps).
        #[arg(long)]
        served: bool,
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
    /// Take a repository off the running server. Its pages then answer 404.
    ///
    /// The opposite of serving a repository while a server already runs: the server on
    /// `server.addr` is found and checked the same way. The on-disk index stays, so serving it
    /// again is fast. The last repository stays served.
    Remove {
        /// A path inside the repository, or its URL name. Default: the current repository.
        target: Option<String>,
    },
    /// Register (or with `--uninstall` remove) this program as the `gitterrace://` link handler.
    ///
    /// Opt-in. macOS writes `~/Applications/gitterrace-open.app`, Linux a `.desktop` file, and
    /// WSL and Windows only print the steps. It prints exactly what it wrote.
    InstallHandler {
        /// Remove what a previous install wrote.
        #[arg(long)]
        uninstall: bool,
    },
    /// Write a documented `config.toml` and the pre-filled `actions.toml` to the config folder.
    ///
    /// `config.toml` is a template with every setting commented out at its default. `actions.toml`
    /// has the Send to targets and example actions. Those that need your own settings start
    /// switched off. An existing file is kept as it is. To start over, move the file away and
    /// run this again.
    Init,
    /// Write a static copy of the site to a folder, for sharing or hosting.
    Export {
        /// Folder to write into (created if missing). Default: `./gitterrace-export`. Give it
        /// first, before any repositories: `export site repo1 repo2`. A folder an earlier
        /// export made is replaced whole. Any other non-empty folder is refused without
        /// `--force`, and so are a symlink and a folder holding a repository being exported.
        out: Option<PathBuf>,
        /// Write into a non-empty folder that no earlier export made. Its files are kept, a
        /// file in the site's way is replaced, a folder in its way fails the export. Later
        /// exports replace only what an export put there.
        #[arg(long)]
        force: bool,
        /// Newest commits to include as pages.
        #[arg(long, value_name = "N", default_value_t = 500)]
        commits: usize,
        /// Largest file to include as a page, in KiB. Larger files are left out of the export.
        #[arg(long, value_name = "KIB", default_value_t = 512)]
        file_kb: u64,
        /// Most pages to write, folded diff fragments included. Overrides
        /// `limits.export_pages`. Unset, every page in scope is written. Past a set cap the
        /// export stops and writes nothing.
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u64).range(1..=10_000_000)
        )]
        export_pages: Option<u64>,
        /// Print no progress. Progress goes to stderr: one updating line on a terminal, a
        /// line every few seconds otherwise.
        #[arg(long, short)]
        quiet: bool,
        /// Credit a repository that is someone else's project (a demo), as `name=url`: its
        /// pages name and link it in the snapshot banner. `name` is the repository's folder
        /// name. Repeat it for several repositories.
        #[arg(long, value_name = "NAME=URL")]
        project: Vec<String>,
    },
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    /// A flag that names a config key is that key with dashes, and no flag says `max`. A flag
    /// naming a key under any other spelling, or a new `--max-*`, fails here.
    #[test]
    fn flags_mirror_their_config_keys() {
        let keys = gitterrace_config::keys();
        let mut bad = Vec::new();
        let root = super::Cli::command();
        let mut cmds = vec![root.clone()];
        cmds.extend(root.get_subcommands().cloned());
        for c in &cmds {
            for a in c.get_arguments() {
                let Some(long) = a.get_long() else { continue };
                let help = a.get_help().map(ToString::to_string).unwrap_or_default();
                if long.split('-').any(|w| w == "max" || w == "min") {
                    bad.push(format!("--{long}: says max"));
                }
                // `Overrides `section.key`` in the help names the key the flag sets.
                for k in keys
                    .iter()
                    .filter(|k| help.contains(&format!("`{}`", k.id)))
                {
                    if help.contains("Overrides")
                        && !gitterrace_config::naming::flag_fits(long, k.id)
                    {
                        bad.push(format!("--{long} sets {} against the naming rule", k.id));
                    }
                }
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}
