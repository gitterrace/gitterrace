//! The slim bar on top of every site page, from `templates/bar.html`: brand, then Docs, the demo
//! when there is one and the source link, each an icon and a word, and the theme button. It
//! follows the page's color scheme, light in light mode and dark in dark mode, as the app's
//! header does.

use askama::Template;

/// The source host's mark (16x16 path), the source link in the bar.
const GH_MARK: &str = "M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38\n\
0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01\n\
-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64\n\
-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0\n\
1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87\n\
3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016\n\
8c0-4.42-3.58-8-8-8z";

/// The Docs icon, an open book in the theme button's line style. The bar's Docs menu draws
/// it too, so the link and the menu that stands in for it look the same.
pub(crate) const DOCS_ICON: &str = "<svg class=\"ico\" viewBox=\"0 0 24 24\" width=\"18\" \n\
height=\"18\" aria-hidden=\"true\"><path d=\"M12 6.5C10.5 5.3 8.3 4.75 4 4.75v13c4.3 0 6.5.55 \n\
8 1.75 1.5-1.2 3.7-1.75 8-1.75v-13c-4.3 0-6.5.55-8 1.75zM12 6.5v13\"/></svg>";

/// The demo icon, a play triangle in a circle: the demo runs the app.
const DEMO_ICON: &str = "<svg class=\"ico\" viewBox=\"0 0 24 24\" width=\"18\" height=\"18\" \n\
aria-hidden=\"true\"><circle cx=\"12\" cy=\"12\" r=\"8.5\"/><path d=\"M10.25 8.9v6.2L15.1 \n\
12z\"/></svg>";

/// The theme button, the app's icons: half disc for system, sun, moon. Each viewBox is cropped
/// to its own ink plus half the stroke, so at 18 px every icon fills the box as the source-link
/// mark beside it does. Its label names the scheme now and the one a press picks.
const THEME_BUTTON: &str = "<button class=\"theme\" type=\"button\" id=\"theme\" \n\
aria-label=\"Theme: system, click for light\"\n\
title=\"Theme: system, click for light\">\n\
<svg class=\"i-auto\" viewBox=\"3.2 3.2 17.6 17.6\" aria-hidden=\"true\"><circle cx=\"12\" \n\
cy=\"12\" r=\"8\"/><path d=\"M12 4a8 8 0 0 0 0 16z\" class=\"fill\"/></svg>\n\
<svg class=\"i-light\" viewBox=\"1.2 1.2 21.6 21.6\" aria-hidden=\"true\"><circle cx=\"12\" \n\
cy=\"12\" r=\"4\"/><path d=\"M12 2v2.5M12 19.5V22M2 12h2.5M19.5 12H22M4.9 4.9l1.8 1.8M17.3 \n\
17.3l1.8 1.8M4.9 19.1l1.8-1.8M17.3 6.7l1.8-1.8\"/></svg>\n\
<svg class=\"i-dark\" viewBox=\"3.85 3.2 16.95 16.95\" aria-hidden=\"true\"><path d=\"M20 14.5A8 \n\
8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z\"/></svg></button>";

#[derive(Template)]
#[template(path = "bar.html")]
struct Bar<'a> {
    name: &'a str,
    repo: &'a str,
    demo: &'a str,
    gh_mark: &'a str,
    docs_icon: &'a str,
    demo_icon: &'a str,
    theme_button: &'a str,
    docs_menu: &'a str,
    current: bool,
}

/// The bar for the site `name` with its source at `repo`. `demo` is the live demo's URL, empty
/// when there is none, and then the bar has no Demo link. `docs_menu` is a docs page's folded
/// docs list, which stands in for the Docs link on a narrow screen, empty off the docs.
/// `current` marks the Docs link as the current page's section, on every page under /docs/.
pub(crate) fn bar(name: &str, repo: &str, demo: &str, docs_menu: &str, current: bool) -> String {
    let mut out = Bar {
        name,
        repo,
        demo,
        gh_mark: GH_MARK,
        docs_icon: DOCS_ICON,
        demo_icon: DEMO_ICON,
        theme_button: THEME_BUTTON,
        docs_menu,
        current,
    }
    .render()
    .unwrap_or_default();
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::{DOCS_ICON, bar};

    #[test]
    fn the_demo_link_is_there_only_with_a_demo() {
        let none = bar("r", "https://github.com/o/r", "", "", false);
        assert!(!none.contains("class=\"live\""), "{none}");
        let one = bar(
            "r",
            "https://github.com/o/r",
            "https://demo.example",
            "",
            false,
        );
        assert!(
            one.contains("<a class=\"live\" href=\"https://demo.example\""),
            "{one}"
        );
        // The demo is a link among the links, between Docs and the source, not a button.
        assert!(
            one.find("class=\"docs\"") < one.find("class=\"live\"")
                && one.find("class=\"live\"") < one.find("class=\"gh\""),
            "{one}"
        );
        for b in [none, one] {
            assert_eq!(b.matches("<header class=\"top\">").count(), 1);
            assert_eq!(b.matches("<nav ").count(), 1);
            assert!(b.contains("id=\"theme\"") && b.contains("class=\"gh\""));
            assert!(b.contains(&format!(
                "<a class=\"docs\" href=\"/docs/\">{DOCS_ICON}<span>Docs</span></a>"
            )));
        }
    }

    #[test]
    fn the_docs_link_is_current_only_under_the_docs() {
        let off = bar("r", "https://github.com/o/r", "", "", false);
        let on = bar("r", "https://github.com/o/r", "", "", true);
        assert!(!off.contains("aria-current"), "{off}");
        assert!(
            on.contains(&format!(
                "href=\"/docs/\" aria-current=\"page\">{DOCS_ICON}<span>Docs</span></a>"
            )),
            "{on}"
        );
    }

    #[test]
    fn the_name_and_urls_are_escaped() {
        let b = bar("a<b", "https://x/\"y", "", "", false);
        assert!(!b.contains("a<b") && !b.contains("x/\"y"), "{b}");
    }
}
