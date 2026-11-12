//! A reader that refuses a line longer than a limit.
//!
//! The transport reads one JSON-RPC message per line into a buffer that has no bound of its
//! own, so a peer that sends bytes without a newline would grow the heap until the process
//! dies. Wrapping the reader turns that into a read error after `limit` bytes of one line.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

/// Longest accepted message line on stdin. A tool call is a few hundred bytes. Not
/// configurable: the cap only refuses input no client sends.
pub(super) const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

pub(super) struct LineBound<R> {
    inner: R,
    limit: usize,
    run: usize,
}

impl<R> LineBound<R> {
    pub(super) const fn new(inner: R, limit: usize) -> Self {
        Self {
            inner,
            limit,
            run: 0,
        }
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for LineBound<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if !matches!(polled, Poll::Ready(Ok(()))) {
            return polled;
        }
        let mut run = self.run;
        let limit = self.limit;
        for &b in &buf.filled()[before..] {
            run = if b == b'\n' { 0 } else { run + 1 };
            if run > limit {
                // Hand the bytes back unread so the caller's buffer stays within the limit.
                buf.set_filled(before);
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "mcp message line exceeds the size limit",
                )));
            }
        }
        self.run = run;
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    #[tokio::test]
    async fn an_oversized_line_is_refused_without_buffering_it_whole() {
        let (mut tx, rx) = tokio::io::duplex(64 * 1024);
        let writer = tokio::spawn(async move {
            let chunk = vec![b'x'; 64 * 1024];
            // Stops when the reader hangs up: the write fails once the pipe closes.
            for _ in 0..(5 * 1024 * 1024 / chunk.len()) {
                if tx.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        });
        let mut r = BufReader::new(LineBound::new(rx, MAX_LINE_BYTES));
        let mut line = Vec::new();
        let err = r.read_until(b'\n', &mut line).await.expect_err("refused");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(
            line.len() <= MAX_LINE_BYTES,
            "buffered {} bytes",
            line.len()
        );
        drop(r);
        writer.await.expect("writer");
    }

    #[tokio::test]
    async fn many_short_lines_pass_past_the_limit_in_total() {
        let (mut tx, rx) = tokio::io::duplex(1024);
        let writer = tokio::spawn(async move {
            for _ in 0..100 {
                tx.write_all(b"0123456789\n").await.expect("write");
            }
        });
        let mut r = BufReader::new(LineBound::new(rx, 16));
        let mut n = 0;
        loop {
            let mut line = Vec::new();
            if r.read_until(b'\n', &mut line).await.expect("read") == 0 {
                break;
            }
            n += 1;
        }
        assert_eq!(n, 100);
        writer.await.expect("writer");
    }
}
