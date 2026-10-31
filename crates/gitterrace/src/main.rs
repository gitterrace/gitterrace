use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;

/// Browse a git repository and its stats in the browser.
#[derive(Parser)]
#[command(version = option_env!("GITTERRACE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION")))]
struct Cli {
    /// Repository to serve (any path inside it).
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Address to listen on. Local only by default.
    #[arg(long, default_value = "127.0.0.1:4242")]
    addr: SocketAddr,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();
    let cli = Cli::parse();
    gitterrace_web::serve(cli.path, cli.addr).await?;
    Ok(())
}
