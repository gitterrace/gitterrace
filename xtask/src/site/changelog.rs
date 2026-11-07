//! `CHANGELOG.md` as the page `/changelog/`. Each release heading gets the id `v<version>`, so
//! the footer and release notes can link to `/changelog/#v0.3.0`. The `### Added` style headings
//! repeat in every release, so they take the release's id as a prefix to stay unique.

use crate::out::Out;
use crate::site::docs::{CHANGELOG, Links, Site, body, shell, slug, source_link};

/// The site path of the page.
pub const PATH: &str = "changelog/";

/// The id of a release heading such as `[0.3.0] - 2026-01-02` or `[Unreleased]`.
fn release_id(heading: &str) -> String {
    let version = heading
        .trim_start_matches('[')
        .split(']')
        .next()
        .unwrap_or(heading);
    let bare = version.strip_prefix('v').unwrap_or(version);
    if bare.starts_with(|c: char| c.is_ascii_digit()) {
        format!("v{bare}")
    } else {
        slug(version)
    }
}

/// The footer's link for version `ver`: its release heading when the changelog has one, else
/// the page, so the link and the id come from the same rule and never point at nothing.
pub fn version_href(text: &str, ver: &str) -> String {
    let heading = format!("## [{ver}]");
    if text.lines().any(|l| l.starts_with(&heading)) {
        format!("{PATH}#{}", release_id(&format!("[{ver}]")))
    } else {
        PATH.to_owned()
    }
}

/// The changelog's HTML, with its heading ids.
fn render(text: &str, links: &Links) -> String {
    let mut release = String::new();
    body(text, CHANGELOG, links, &mut |level, h| match level {
        2 => {
            release = release_id(h);
            release.clone()
        }
        3 if !release.is_empty() => format!("{release}-{}", slug(h)),
        _ => slug(h),
    })
}

/// The changelog page.
pub(super) fn page(site: &Site, links: &Links, text: &str) -> Out {
    let inner = render(text, links);
    Out::file(
        format!("site/{PATH}index.html"),
        shell(
            site,
            "Changelog",
            PATH,
            &inner,
            &source_link(site, CHANGELOG),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_release_heading_is_its_version_as_anchor() {
        let text = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- a\n\n\
                    ## [0.3.0] - 2026-01-02\n\n### Added\n\n- b\n";
        let html = render(text, &Links::new("https://r"));
        assert!(html.contains("<h2 id=\"v0.3.0\">"), "{html}");
        assert!(html.contains("<h2 id=\"unreleased\">"), "{html}");
        assert!(html.contains("<h3 id=\"v0.3.0-added\">"), "{html}");
        assert!(html.contains("<h3 id=\"unreleased-added\">"), "{html}");
    }

    #[test]
    fn the_version_link_names_a_release_only_when_it_exists() {
        let text = "## [0.3.0] - 2026-01-02\n";
        assert_eq!(version_href(text, "0.3.0"), "changelog/#v0.3.0");
        assert_eq!(version_href(text, "0.4.0"), "changelog/");
    }
}
