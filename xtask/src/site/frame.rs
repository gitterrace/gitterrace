//! The frame every page of the site shares, from `templates/page.html` and
//! `templates/footer.html`: the head, the bar, the one grid (docs list, text, "On this
//! page") and the footer. The landing page, the docs, the changelog, the legal pages and the
//! 404 all go through [`page`], so none can drift from the others and the text column starts
//! at the same place on every page.

use askama::Template;

use crate::site::{MENU_JS, THEME_INIT_JS, THEME_TOGGLE_JS, bar};

/// What a page shows, the parts only it has. Every HTML value is markup the site built.
#[derive(Default)]
pub(crate) struct Body<'a> {
    /// The `<title>`, without the site name, which the frame appends. Empty on the landing
    /// page, whose title is the name alone.
    pub title: &'a str,
    /// The meta description: one sentence of what the page covers.
    pub desc: &'a str,
    /// The site path, `docs/cli/`, empty for the home page.
    pub path: &'a str,
    /// A mono label above the h1 (the legal pages name what they are), empty for none.
    pub kicker: &'a str,
    /// The page's own markup, its h1 first.
    pub inner: &'a str,
    /// The docs list on the left, empty off the docs.
    pub docnav: &'a str,
    /// The bar's Docs menu, the docs list folded for a narrow screen, empty off the docs.
    pub docs_menu: &'a str,
    /// The "On this page" menu above the text on a narrow screen, empty for none.
    pub menus: &'a str,
    /// Previous and next at the end of a docs page, empty for none.
    pub pager: &'a str,
    /// The "On this page" rail, empty for none.
    pub rail: &'a str,
    /// A docs page's source file and whether it is generated, for the footer's edit link.
    pub edit: Option<(&'a str, bool)>,
    /// Extra inline scripts after the theme button's; each is hashed into the CSP.
    pub scripts: Vec<String>,
    /// Keep the page out of search results (the 404).
    pub noindex: bool,
}

/// The site-wide facts the frame needs.
pub(crate) struct Site<'a> {
    pub home: &'a str,
    pub repo: &'a str,
    pub name: &'a str,
    pub demo: &'a str,
    pub license: &'a str,
    pub version: &'a str,
    /// Where the footer's version links: its changelog release, or the page.
    pub version_href: &'a str,
}

#[derive(Template)]
#[template(path = "page.html")]
struct Page<'a> {
    title: &'a str,
    desc: &'a str,
    home: &'a str,
    path: &'a str,
    name: &'a str,
    noindex: bool,
    theme_init: &'a str,
    bar: &'a str,
    docnav: &'a str,
    menus: &'a str,
    kicker: &'a str,
    inner: &'a str,
    pager: &'a str,
    rail: &'a str,
    footer: &'a str,
    scripts: &'a [String],
}

#[derive(Template)]
#[template(path = "footer.html")]
struct Footer<'a> {
    repo: &'a str,
    legal: Vec<(String, &'a str)>,
    license: &'a str,
    version: &'a str,
    version_href: &'a str,
    edit: &'a str,
    generated: bool,
}

/// The footer of every page: the shared links, then a docs page's source.
pub(crate) fn footer(site: &Site, edit: Option<(&str, bool)>) -> String {
    let legal = crate::docs_index::LEGAL
        .iter()
        .map(|(rel, label)| (crate::site::docs::legal_path(rel), *label))
        .collect();
    let mut out = Footer {
        repo: site.repo,
        legal,
        license: site.license,
        version: site.version,
        version_href: site.version_href,
        edit: edit.map_or("", |e| e.0),
        generated: edit.is_some_and(|e| e.1),
    }
    .render()
    .unwrap_or_default();
    out.push('\n');
    out
}

/// A whole page in the shared frame.
pub(crate) fn page(site: &Site, body: &Body) -> String {
    let title = if body.title.is_empty() {
        site.name.to_owned()
    } else {
        format!("{} - {}", body.title, site.name)
    };
    let mut scripts = vec![THEME_TOGGLE_JS.to_owned()];
    if !body.docs_menu.is_empty() {
        scripts.push(MENU_JS.to_owned());
    }
    scripts.extend(body.scripts.iter().cloned());
    let mut out = Page {
        title: &title,
        desc: body.desc,
        home: site.home,
        path: body.path,
        name: site.name,
        noindex: body.noindex,
        theme_init: THEME_INIT_JS,
        bar: &bar::bar(site.name, site.repo, site.demo, body.docs_menu),
        docnav: body.docnav,
        menus: body.menus,
        kicker: body.kicker,
        inner: body.inner,
        pager: body.pager,
        rail: body.rail,
        footer: &footer(site, body.edit),
        scripts: &scripts,
    }
    .render()
    .unwrap_or_default();
    out.push('\n');
    // The page is a committed source file, held to the line width like the rest.
    crate::site::fit::fit(&out)
}

/// `html` with its tags packed into lines at most 100 wide. A line breaks only between a `>`
/// and an opening `<`, where no whitespace is added and none is lost, so a closing tag stays on
/// the line of what it closes. Lists the site builds in one go (the docs list, the menus) stay
/// short and within the line width the pages are held to. One run without such a break point
/// that is wider than 100 is left whole.
pub(crate) fn pack(html: &str) -> String {
    let mut out = String::new();
    let mut line = 0;
    let mut rest = html;
    while !rest.is_empty() {
        // The next run ends before the first `<` that opens a tag right after a `>`.
        let end = rest
            .match_indices("><")
            .map(|(i, _)| i + 1)
            .find(|&i| !rest[i..].starts_with("</"))
            .unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        let n = run.chars().count();
        if line > 0 && line + n > 100 {
            out.push('\n');
            line = 0;
        }
        out.push_str(run);
        line += n;
        rest = tail;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    fn site() -> Site<'static> {
        Site {
            home: "https://h.example",
            repo: "https://github.com/o/r",
            name: "r",
            demo: "",
            license: "MIT",
            version: "0.1.0",
            version_href: "changelog/",
        }
    }

    #[test]
    fn every_page_gets_the_same_footer_and_a_docs_page_adds_its_edit_link() {
        let a = footer(&site(), None);
        let b = footer(&site(), Some(("docs/why.md", false)));
        let c = footer(&site(), Some(("docs/cli.md", true)));
        assert!(b.starts_with(a.trim_end_matches("</footer>\n")), "{a}\n{b}");
        assert!(!a.contains("class=\"edit\""), "{a}");
        assert!(b.contains("/edit/main/docs/why.md\">Edit on GitHub"), "{b}");
        assert!(
            c.contains("/blob/main/docs/cli.md\">View source on GitHub"),
            "{c}"
        );
    }

    #[test]
    fn values_are_escaped_and_the_title_carries_the_name() {
        let p = page(
            &site(),
            &Body {
                title: "a<b",
                desc: "d\"e",
                inner: "<h1>x</h1>",
                ..Body::default()
            },
        );
        assert!(!p.contains("a<b") && p.contains(" - r</title>"), "{p}");
        assert!(!p.contains("d\"e"), "{p}");
        assert!(p.contains("<h1>x</h1>"), "{p}");
    }

    #[test]
    fn packing_breaks_only_between_tags_and_keeps_the_width() {
        let list = (0..30).fold(String::new(), |mut s, i| {
            let _ = write!(s, "<li><a href=\"/{i}/\">Item {i}</a></li>");
            s
        });
        let packed = pack(&list);
        assert_eq!(packed.replace('\n', ""), list);
        assert!(packed.lines().all(|l| l.chars().count() <= 100), "{packed}");
        assert!(
            packed
                .lines()
                .skip(1)
                .all(|l| l.starts_with('<') && !l.starts_with("</")),
            "{packed}"
        );
    }
}
