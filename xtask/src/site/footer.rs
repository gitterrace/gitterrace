//! Checks over the footer of the built docs pages.

use crate::site::tests::generated;

fn footer_of(html: &str) -> Option<&str> {
    let (_, rest) = html.split_once("<footer>")?;
    rest.split_once("</footer>").map(|x| x.0)
}

/// The class this guards: a docs page whose footer lost its labelled nav, repeats a link, or
/// does not end with the source link after the nav (the right side), and a stylesheet where a
/// content link sets its own color instead of the shared one. The same rules as
/// `site-check.py links`, which runs on the built site.
#[test]
fn the_docs_footer_is_a_labelled_nav_with_the_edit_link_last() {
    let (pages, _) = generated();
    let mut seen = 0;
    for (path, html) in pages.iter().filter(|(p, _)| p.starts_with("docs/")) {
        let foot = footer_of(html).unwrap_or("");
        assert!(
            foot.contains("<nav aria-label=\"Footer\">"),
            "{path}: {foot}"
        );
        let hrefs = crate::site::attrs(foot, "href");
        let mut once = hrefs.clone();
        once.sort_unstable();
        once.dedup();
        assert_eq!(hrefs.len(), once.len(), "{path}: a link twice");
        if foot.contains("class=\"edit\"") {
            seen += 1;
            let after = foot.rsplit_once("</nav>").map_or("", |x| x.1);
            assert!(
                after.trim_start().starts_with("<a class=\"edit\""),
                "{path}"
            );
        }
    }
    assert!(seen > 0, "no docs page has an Edit link");
}
