//! Setup the page tests share.

#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::collections::BTreeMap;
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

/// Builds the insights cache of `repo` up to HEAD, as an indexer would.
pub(crate) fn build_index(repo: &Path) {
    let r = gitterrace_git::Repo::discover(repo).expect("repo");
    let index = gitterrace_index::Index::open(&r).expect("index");
    index.update(&r, "HEAD", &|_| {}).expect("build");
}

/// Every file of the cache folder `dir` with its bytes, but the ones `skip` names.
pub(crate) fn snapshot(dir: &Path, skip: impl Fn(&str) -> bool) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir)
        .expect("dir")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !skip(n))
        .map(|n| {
            let b = std::fs::read(dir.join(&n)).unwrap_or_default();
            (n, b)
        })
        .collect()
}
