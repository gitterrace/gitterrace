//! Every absolute link to the site, from anywhere in the repository (the app's code and
//! templates, the README, the docs, scripts), resolves against the site this crate generates:
//! the page exists and so does the heading its `#anchor` names. The site's own relative links
//! are checked in tests.rs; an absolute one is checked nowhere else.

use crate::out::Out;
use crate::site::tests::{generated, unresolved};
use std::fmt::Write as _;

const SITE: &str = "https://gitterrace.dev";

/// Every `https://gitterrace.dev/...` link in `text`, as the site path after the host.
fn site_links(text: &str) -> Vec<String> {
    text.match_indices(SITE)
        .filter_map(|(at, _)| {
            let rest = &text[at + SITE.len()..];
            let end = rest
                .find(|c: char| c == '"' || c == '\'' || c == ')' || c.is_whitespace())
                .unwrap_or(rest.len());
            let link = &rest[..end];
            link.starts_with('/').then(|| link.to_owned())
        })
        .collect()
}

/// (file, site path) of every site link in a tracked text file. Generated outputs (goldens,
/// bench results, the changelog) quote links rather than make them, and this file plants dead
/// ones on purpose.
fn repo_links() -> Vec<(String, String)> {
    let root = crate::repo_root();
    let out = gitterrace_testkit::git(&root, &["ls-files", "-z"]);
    let skip = |f: &str| {
        f.contains("/golden/")
            || f.starts_with("tests/bench/results/")
            || f == "CHANGELOG.md"
            || f.ends_with("site/repo_links.rs")
    };
    let mut links = Vec::new();
    for f in out.split('\0') {
        if f.is_empty() || skip(f) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(f)) else {
            continue;
        };
        links.extend(site_links(&text).into_iter().map(|l| (f.to_owned(), l)));
    }
    links
}

/// The unresolved ones among `links`, checked as if a site page linked them. A path the
/// generated `_redirects` serves (the installer) counts as a file.
fn dead(pages: &[(String, String)], files: &[String], links: &[(String, String)]) -> Vec<String> {
    let mut files = files.to_vec();
    files.extend(redirect_sources(&redirects_file()));
    let files = &files;
    let mut probe = pages.to_vec();
    let body = links.iter().fold(String::new(), |mut s, (_, l)| {
        let _ = write!(s, "<a href=\"{l}\">x</a>");
        s
    });
    probe.push(("app-links.html".to_owned(), body));
    let bad = unresolved(&probe, files);
    links
        .iter()
        .filter(|(_, l)| {
            bad.iter()
                .any(|b| b.starts_with(&format!("app-links.html: {l}:")))
        })
        .map(|(f, l)| format!("{f}: {SITE}{l}"))
        .collect()
}

/// The generated `_redirects` body.
fn redirects_file() -> String {
    let root = crate::repo_root();
    let ws = crate::facts::load(&root).expect("facts");
    crate::site::render(&root, &ws)
        .expect("render")
        .into_iter()
        .find_map(|o| match o {
            Out::File { path, body } if path.ends_with("/_redirects") => Some(body),
            _ => None,
        })
        .unwrap_or_default()
}

/// The site paths a `_redirects` body answers, without the leading `/`.
fn redirect_sources(body: &str) -> Vec<String> {
    body.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            l.split_whitespace()
                .next()?
                .strip_prefix('/')
                .map(str::to_owned)
        })
        .collect()
}

/// The class this guards: a link from anywhere in the repository to a site page or heading
/// that does not exist, as the log filter's help did after its heading was renamed.
#[test]
fn every_site_link_in_the_repository_resolves() {
    let (pages, files) = generated();
    let links = repo_links();
    assert!(links.len() > 5, "only {} site links found", links.len());
    let bad = dead(&pages, &files, &links);
    assert!(bad.is_empty(), "dead site links:\n{}", bad.join("\n"));
}

#[test]
fn a_planted_dead_site_link_is_reported() {
    let (pages, files) = generated();
    let links = vec![
        ("a.rs".to_owned(), "/docs/using/#no-such-heading".to_owned()),
        ("b.rs".to_owned(), "/docs/no-such-page/".to_owned()),
        ("c.rs".to_owned(), "/docs/using/".to_owned()),
        ("d.rs".to_owned(), "/install.sh".to_owned()),
        ("e.rs".to_owned(), "/install.bash".to_owned()),
    ];
    let bad = dead(&pages, &files, &links).join("\n");
    assert!(
        bad.contains("a.rs:") && bad.contains("b.rs:") && bad.contains("e.rs:"),
        "{bad}"
    );
    assert!(!bad.contains("c.rs:") && !bad.contains("d.rs:"), "{bad}");
    assert_eq!(
        site_links("x = \"https://gitterrace.dev/docs/using/#a\"; y = 'https://gitterrace.dev'"),
        ["/docs/using/#a"]
    );
}
