use std::{net::SocketAddr, path::PathBuf};

use clap::{Parser, Subcommand};

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
    }
    Ok(())
}
