//! The "about" documents of a tree (README, licence, security policy and so on), shown as tabs
//! in the card under the file list. Which files count is decided here and nowhere else.

use gitterrace_git::{TreeEntry, TreeEntryKind};

use crate::{fmt, markdown};

/// The kinds, in tab order: slug, label, and the upper-case stems in order of preference.
const KINDS: [(&str, &str, &[&str]); 6] = [
    ("readme", "README", &["README"]),
    ("license", "LICENSE", &["LICENSE", "LICENCE", "COPYING"]),
    ("contributing", "CONTRIBUTING", &["CONTRIBUTING"]),
    ("security", "SECURITY", &["SECURITY"]),
    ("conduct", "CODE OF CONDUCT", &["CODE_OF_CONDUCT"]),
    ("changelog", "CHANGELOG", &["CHANGELOG"]),
];
/// Extensions in order of preference; the empty one is a bare name.
const EXTS: [&str; 4] = [".md", ".markdown", ".txt", ""];

/// One present document: the kind's slug and label, and the file name inside the tree.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Doc {
    pub(crate) slug: &'static str,
    pub(crate) label: &'static str,
    pub(crate) file: String,
}

/// A tab of the card.
pub(crate) struct DocTab {
    pub(crate) href: String,
    pub(crate) label: &'static str,
    pub(crate) on: bool,
}

/// The card: the tab row (empty when one document is present) and the active document.
pub(crate) struct About {
    pub(crate) label: &'static str,
    pub(crate) tabs: Vec<DocTab>,
    pub(crate) html: String,
    pub(crate) markdown: bool,
}

/// Preference of `name` for a kind: (stem, extension, not upper-case), lower is better.
fn rank(name: &str, stems: &[&str]) -> Option<(usize, usize, bool)> {
    let upper = name.to_ascii_uppercase();
    EXTS.iter().enumerate().find_map(|(e, ext)| {
        let stem = upper.strip_suffix(&ext.to_ascii_uppercase())?;
        let s = stems.iter().position(|k| *k == stem)?;
        Some((s, e, name != upper))
    })
}

/// The present documents among `entries` (one directory), in tab order. Only regular files
/// count: a folder or symlink called LICENSE is ignored.
pub(crate) fn find(entries: &[TreeEntry]) -> Vec<Doc> {
    KINDS
        .iter()
        .filter_map(|(slug, label, stems)| {
            entries
                .iter()
                .filter(|e| e.kind == TreeEntryKind::File)
                .filter_map(|e| Some((rank(&e.name, stems)?, &e.name)))
                .min()
                .map(|(_, name)| Doc {
                    slug,
                    label,
                    file: name.clone(),
                })
        })
        .collect()
}

/// Builds the card for `docs`, showing the one whose slug is `want` (else the first). `page` is
/// the page's own path, to which `?doc=` is appended. `None` when there is no document.
pub(crate) fn card(
    a: &crate::app::App,
    id: &str,
    rev: &str,
    dir: &str,
    page: &str,
    docs: &[Doc],
    want: Option<&str>,
) -> Option<About> {
    let active = docs
        .iter()
        .find(|d| Some(d.slug) == want)
        .or_else(|| docs.first())?;
    let file = if dir.is_empty() {
        active.file.clone()
    } else {
        format!("{dir}/{}", active.file)
    };
    let is_md = {
        let l = active.file.to_ascii_lowercase();
        l.ends_with(".md") || l.ends_with(".markdown")
    };
    let html = match a.repo.blob(id, &file).ok().and_then(|b| b.text) {
        Some(t) if is_md => markdown::render_with(&t, markdown::Base::new(rev, dir)),
        Some(t) => format!("<pre>{}</pre>", fmt::esc_text(&t)),
        None => "<p class=\"muted\">Too large or not text.</p>".to_owned(),
    };
    let tabs = if docs.len() > 1 {
        docs.iter()
            .map(|d| DocTab {
                href: fmt::href(&format!("{page}?doc={}", d.slug)),
                label: d.label,
                on: d.slug == active.slug,
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(About {
        label: active.label,
        tabs,
        html,
        markdown: is_md,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, kind: TreeEntryKind) -> TreeEntry {
        TreeEntry {
            name: name.into(),
            kind,
            size: None,
        }
    }

    #[test]
    fn fixed_order_case_insensitive_one_per_kind() {
        let f = TreeEntryKind::File;
        let docs = find(&[
            e("contributing.md", f),
            e("SECURITY.md", f),
            e("LICENSE.txt", f),
            e("LICENSE", f),
            e("Readme.markdown", f),
            e("README.txt", f),
            e("a.rs", f),
        ]);
        let got: Vec<_> = docs.iter().map(|d| (d.slug, d.file.as_str())).collect();
        assert_eq!(
            got,
            [
                ("readme", "Readme.markdown"),
                ("license", "LICENSE.txt"),
                ("contributing", "contributing.md"),
                ("security", "SECURITY.md"),
            ]
        );
    }

    #[test]
    fn folders_symlinks_and_submodules_are_ignored() {
        let docs = find(&[
            e("LICENSE", TreeEntryKind::Dir),
            e("SECURITY.md", TreeEntryKind::Symlink),
            e("CHANGELOG", TreeEntryKind::Submodule),
            e("README.md", TreeEntryKind::File),
        ]);
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].slug, "readme");
    }
}
