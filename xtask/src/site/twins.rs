//! Every document the site publishes as a page is also published as the raw markdown it is
//! rendered from, at `<page>index.md`. The page advertises it twice: a `<link rel="alternate">`
//! in its head and a `Link` header from `_headers`. Agents read the markdown instead of
//! parsing the HTML, and `llms.txt` links these files.

use std::fmt::Write as _;

use crate::out::Out;
use crate::site::at;

/// The content type the twins and the `llms` files are served as.
const MARKDOWN: &str = "text/markdown; charset=utf-8";

/// A page of the site that has a markdown file beside it.
pub(super) struct Twin {
    /// The page's site path, `docs/cli/`.
    pub path: String,
    /// The repository file the page is rendered from.
    pub source: String,
    pub title: String,
    /// One sentence of what it covers.
    pub what: String,
    /// The heading `llms.txt` lists it under.
    pub group: String,
    /// The markdown as it is in the repository.
    pub text: String,
}

impl Twin {
    /// The site path of the markdown file: `docs/cli/index.md`.
    pub(super) fn md_path(&self) -> String {
        md_path(&self.path)
    }

    /// The markdown file, as the site serves it.
    pub(super) fn out(&self) -> Out {
        Out::file(at(&self.md_path()), &self.text)
    }
}

/// The site path of the markdown file of the page at `path`.
pub(super) fn md_path(path: &str) -> String {
    format!("{path}index.md")
}

/// The `_headers` blocks of the twins: each page advertises its markdown file in a `Link`
/// header, and the file is served as markdown.
pub(super) fn header_blocks(twins: &[Twin]) -> String {
    let mut out = String::new();
    for t in twins {
        let md = t.md_path();
        let _ = write!(
            out,
            "\n/{}\n  Link: </{md}>; rel=\"alternate\"; type=\"text/markdown\"\n\n/{md}\n  \
             Content-Type: {MARKDOWN}\n",
            t.path
        );
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn twin(path: &str, group: &str) -> Twin {
        Twin {
            path: path.to_owned(),
            source: "docs/x.md".to_owned(),
            title: "X".to_owned(),
            what: "What x covers.".to_owned(),
            group: group.to_owned(),
            text: "---\nname: x\n---\n\n# X\n\nBody.\n".to_owned(),
        }
    }

    #[test]
    fn every_twin_page_advertises_its_markdown_in_a_link_header() {
        let blocks = header_blocks(&[twin("docs/x/", "Start")]);
        let link = "/docs/x/\n  Link: </docs/x/index.md>; rel=\"alternate\"";
        assert!(blocks.contains(link), "{blocks}");
        assert!(blocks.contains("/docs/x/index.md\n  Content-Type: text/markdown"), "{blocks}");
    }

    #[test]
    fn a_twin_is_the_source_text_beside_its_page() {
        let t = twin("docs/x/", "Start");
        assert_eq!(t.md_path(), "docs/x/index.md");
        assert!(matches!(t.out(), Out::File { body, .. } if body == t.text));
    }
}
