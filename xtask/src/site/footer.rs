//! Checks over the footer of the built docs pages.

use crate::site::built::footer_of;
use crate::site::tests::generated;

/// The class this guards: a docs page whose footer lost its two labelled navs (the project on
/// the left, the legal pages last on the right), repeats a link, or breaks the project
/// nav's order: the page's own link right after the repository link, and the version last.
/// The same rules as `site-check.py links`, which runs on the built site.
#[test]
fn the_docs_footer_is_project_then_legal_with_edit_after_source_and_version_last() {
    let (pages, _) = generated();
    let mut seen = 0;
    for (path, html) in pages.iter().filter(|(p, _)| p.starts_with("docs/")) {
        let foot = footer_of(html).unwrap_or("");
        let (project, legal) =
            foot.split_once("<nav aria-label=\"Legal\" class=\"legal\">").unwrap_or_default();
        assert!(
            project.contains("<nav aria-label=\"Project\">") && legal.contains("/legal/"),
            "{path}: {foot}"
        );
        assert!(!project.contains("/legal/"), "{path}: a legal link on the left");
        let hrefs = crate::site::attrs(foot, "href");
        let mut once = hrefs.clone();
        once.sort_unstable();
        once.dedup();
        assert_eq!(hrefs.len(), once.len(), "{path}: a link twice");
        if foot.contains("class=\"edit\"") {
            seen += 1;
            let links: Vec<&str> = project.split("<a ").skip(1).collect();
            let n = links.len();
            let edit = links.iter().position(|l| l.starts_with("class=\"edit\""));
            let source = links.iter().position(|l| l.starts_with("class=\"repo\""));
            assert!(
                n >= 2
                    && links[n - 1].starts_with("class=\"version\"")
                    && source.zip(edit).is_some_and(|(s, e)| e == s + 1),
                "{path}: {project}"
            );
        }
    }
    assert!(seen > 0, "no docs page has an Edit link");
}
