//! Setup the page tests share.

#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::path::Path;

/// A router over `repo` whose index is built, ready for pages.
pub(crate) async fn ready_router(repo: impl AsRef<Path>) -> axum::Router {
    let router = gitterrace_web::app_router(repo).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    router
}
