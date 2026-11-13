//! Where the crawl starts: the seeds every exported repository is crawled from.

use std::collections::VecDeque;

/// Pages the crawl starts from, beside every page the server prewarms; each is exported
/// whatever links point at it.
const SEEDS: [&str; 2] = ["/refs", "/log/HEAD"];

/// Every seed: the extra ones, the server's prewarmed pages (every unfiltered insights page
/// `cfg` serves), the assets `app.js` adds on first use, and the diagram library when diagrams
/// are on: a script loads each by name, so the crawl finds no link to it.
pub(super) fn seeds(cfg: &gitterrace_config::Config) -> impl Iterator<Item = &'static str> {
    let on = cfg.feature_on(gitterrace_config::Feature::Mermaid, None);
    let warm = gitterrace_web::warm_pages(cfg);
    SEEDS
        .into_iter()
        .chain(warm)
        .chain(gitterrace_web::LAZY_ASSETS)
        .chain(on.then_some(gitterrace_web::MERMAID_JS))
}

/// Queues `site`'s seeds. A static file is served at the root, so it is queued once, bare.
pub(super) fn queue_seeds(
    queue: &mut VecDeque<String>,
    site: &str,
    cfg: &gitterrace_config::Config,
) {
    for seed in seeds(cfg) {
        if !seed.starts_with("/static/") {
            queue.push_back(format!("{site}{seed}"));
        } else if !queue.iter().any(|q| q == seed) {
            queue.push_back(seed.to_owned());
        }
    }
}
