//! The link from an insights hint to the rule that counts its numbers. Every hint that names a
//! rule goes through [`link`], so the address of the docs page lives in one place and the test
//! in `tests/hint_rule_links.rs` can check every anchor against the site.

/// Where the counting page lives on the site.
pub const COUNTING_PAGE: &str = "https://gitterrace.dev/docs/counting/";

pub const MERGES: &str = "merges";
pub const PEOPLE: &str = "people";
pub const PERIODS: &str = "periods";
pub const CODE_AGE: &str = "code-age-and-releases";
pub const RENAMES: &str = "renames";

/// The address of one rule on the counting page.
pub fn href(anchor: &str) -> String {
    format!("{COUNTING_PAGE}#{anchor}")
}

/// A "How this is counted" link for a hint, as HTML (the anchors are constants, so no input
/// reaches it). The markup is a template, as every page's is.
pub fn link(anchor: &str) -> String {
    use askama::Template as _;
    #[derive(askama::Template)]
    #[template(
        source = r#"<a class="rule" href="{{ href }}" rel="noopener">How this is counted</a>"#,
        ext = "html"
    )]
    struct Link {
        href: String,
    }
    Link { href: href(anchor) }.render().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn read(rel: &str) -> String {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
    }

    /// Every hint link names an anchor that exists as an id on the generated counting page, and
    /// every template reaches the page only through the helper, so no link is typed by hand.
    #[test]
    fn every_hint_link_resolves_to_an_anchor_on_the_site() {
        let page = read("site/docs/counting/index.html");
        let mut used = Vec::new();
        let templates = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
        for e in std::fs::read_dir(templates).expect("templates").flatten() {
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            assert!(
                !text.contains("/docs/counting"),
                "{} links the counting page by hand, use rule::link",
                e.path().display()
            );
            for (i, _) in text.match_indices("rule::link(crate::rule::") {
                let rest = &text[i + "rule::link(crate::rule::".len()..];
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || *c == '_')
                    .collect();
                used.push(name);
            }
        }
        assert!(!used.is_empty(), "no hint links the counting rule");
        let by_name = [
            ("MERGES", MERGES),
            ("PEOPLE", PEOPLE),
            ("PERIODS", PERIODS),
            ("CODE_AGE", CODE_AGE),
            ("RENAMES", RENAMES),
        ];
        for name in used {
            let (_, anchor) = by_name
                .iter()
                .find(|(n, _)| *n == name)
                .unwrap_or_else(|| panic!("{name} is not a known anchor"));
            assert!(
                page.contains(&format!("id=\"{anchor}\"")),
                "hint link {name} points at #{anchor}, which the counting page does not have"
            );
        }
    }

    #[test]
    fn the_link_is_the_counting_page_with_the_anchor() {
        assert_eq!(
            href("merges"),
            "https://gitterrace.dev/docs/counting/#merges"
        );
        assert!(link(MERGES).contains("href=\"https://gitterrace.dev/docs/counting/#merges\""));
    }
}
