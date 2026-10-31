//! HTTP server: server-rendered pages over `gitterrace-git` and `gitterrace-index`.

use std::{net::SocketAddr, path::PathBuf};

/// Starts serving until ctrl-c.
pub async fn serve(_repo_path: PathBuf, _addr: SocketAddr) -> std::io::Result<()> {
    todo!("gitterrace-web")
}
