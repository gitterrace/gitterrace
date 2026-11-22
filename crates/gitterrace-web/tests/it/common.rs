//! Setup the page tests share.

#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::path::Path;

/// A router over `repo` whose index is built, ready for pages.
pub(crate) async fn ready_router(repo: impl AsRef<Path>) -> axum::Router {
    let router = gitterrace_web::app_router(repo).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    router
}

/// The number in the first `<b>` after the row label `label` of a stats page, separators gone.
pub(crate) fn pulse(html: &str, label: &str) -> u64 {
    let rest = [format!("{label}</th>"), format!("{label}</td>"), label.to_owned()]
        .iter()
        .find_map(|end| html.split(end.as_str()).nth(1))
        .expect("label");
    let cell = rest.split("<b").nth(1).expect("bold").split_once('>').expect("tag").1;
    let digits: String = cell.chars().take_while(|c| c.is_ascii_digit() || *c == ',').collect();
    digits.replace(',', "").parse().expect("number")
}

/// The commits of every bar of the per-bucket chart, summed.
pub(crate) fn bars(html: &str) -> u64 {
    html.split("aria-label=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .filter_map(|l| l.split(": ").nth(1)?.split(' ').next()?.parse::<u64>().ok())
        .sum()
}
