//! The accept loop: HTTP/1.1 with a header-read deadline, drained on SIGINT or SIGTERM.

use std::{future::Future, io, time::Duration};

use hyper_util::{
    rt::{TokioIo, TokioTimer},
    server::graceful::GracefulShutdown,
    service::TowerToHyperService,
};
use tokio::net::TcpListener;

/// How long a client may take to send the request line and headers. The request guard
/// only runs once they are parsed, so without this a trickling client holds a socket forever.
/// Not configurable: a real client sends its headers in well under a second.
pub(crate) const HEADER_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// Serves `router` until `stop` resolves, then stops accepting and waits for open
/// connections to finish their responses.
pub(crate) async fn run(
    listener: TcpListener,
    router: axum::Router,
    header_timeout: Duration,
    stop: impl Future<Output = ()>,
) -> io::Result<()> {
    let mut http = hyper::server::conn::http1::Builder::new();
    http.timer(TokioTimer::new())
        .header_read_timeout(header_timeout);
    let graceful = GracefulShutdown::new();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        // Out of descriptors or a reset handshake: back off instead of spinning.
                        tracing::debug!("accept: {e}");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let service = TowerToHyperService::new(router.clone());
                let conn = graceful.watch(http.serve_connection(TokioIo::new(stream), service));
                tokio::spawn(async move {
                    if let Err(e) = conn.await {
                        tracing::debug!("connection: {e}");
                    }
                });
            }
            () = &mut stop => break,
        }
    }
    drop(listener);
    graceful.shutdown().await;
    Ok(())
}

/// Resolves on SIGINT, or SIGTERM on Unix (`docker stop`, `kill`). Registered before the
/// server starts, so a failure to install the handler is an error and not a silent no-op.
#[cfg(unix)]
pub(crate) fn stop_signal() -> io::Result<impl Future<Output = ()>> {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            r = tokio::signal::ctrl_c() => {
                if let Err(e) = r {
                    tracing::warn!("SIGINT handler: {e}");
                    term.recv().await;
                }
            }
            _ = term.recv() => {}
        }
        tracing::info!("shutting down");
    })
}

/// Resolves on Ctrl-C or Ctrl-Break. Windows has no SIGTERM; both handlers are registered
/// before the server starts, as on Unix.
#[cfg(windows)]
pub(crate) fn stop_signal() -> io::Result<impl Future<Output = ()>> {
    let mut c = tokio::signal::windows::ctrl_c()?;
    let mut brk = tokio::signal::windows::ctrl_break()?;
    Ok(async move {
        tokio::select! {
            _ = c.recv() => {}
            _ = brk.recv() => {}
        }
        tracing::info!("shutting down");
    })
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    async fn bound() -> (TcpListener, std::net::SocketAddr) {
        let l = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let a = l.local_addr().expect("addr");
        (l, a)
    }

    #[tokio::test]
    async fn a_client_stalled_in_its_headers_is_cut_after_the_timeout() {
        let (l, addr) = bound().await;
        let router = axum::Router::new().route("/", get(|| async { "ok" }));
        let server = tokio::spawn(run(
            l,
            router,
            Duration::from_secs(1),
            std::future::pending(),
        ));
        let mut s = tokio::net::TcpStream::connect(addr).await.expect("connect");
        s.write_all(b"GET / HTTP/1.1\r\nX-Pad: ")
            .await
            .expect("write");
        let t = Instant::now();
        let mut buf = Vec::new();
        let read = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
        assert!(read.is_ok(), "the server kept a stalled connection open");
        assert!(t.elapsed() >= Duration::from_millis(900));
        server.abort();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sigterm_drains_the_open_request_and_returns_ok() {
        let (l, addr) = bound().await;
        let router = axum::Router::new().route(
            "/slow",
            get(|| async {
                gitterrace_testkit::wait::pause(Duration::from_millis(400)).await;
                "done"
            }),
        );
        let stop = stop_signal().expect("signal handler");
        let server = tokio::spawn(run(l, router, HEADER_READ_TIMEOUT, stop));
        let mut s = tokio::net::TcpStream::connect(addr).await.expect("connect");
        s.write_all(b"GET /slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("write");
        gitterrace_testkit::wait::pause(Duration::from_millis(100)).await;
        rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::TERM)
            .expect("kill");
        let mut buf = String::new();
        s.read_to_string(&mut buf).await.expect("read");
        assert!(buf.starts_with("HTTP/1.1 200"), "{buf}");
        assert!(buf.ends_with("done"), "{buf}");
        let ended = tokio::time::timeout(Duration::from_secs(5), server).await;
        assert!(
            matches!(ended, Ok(Ok(Ok(())))),
            "server did not stop cleanly"
        );
    }
}
