//! Checks over the built site as it is on disk, the tree a deploy ships. The generator's checks see
//! what it writes. These see what is there: a page left behind by a removed document, a link or
//! `#fragment` that names nothing on the built site, and a link into the repository at a file or
//! heading that does not exist there.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};

use crate::docs_index::{normalize, on_site};
use crate::out::Out;
use crate::site::docs::page_path;
use crate::site::{SITE_DIR, guards};

/// Files in the built site that no generator writes but that belong there: the link preview
/// PNG files, which `make site-render` adds before a deploy.
fn made_elsewhere(path: &str) -> bool {
    path.strip_prefix("assets/")
        .is_some_and(|f| !f.contains('/') && has_ext(f, "png"))
}

/// Whether `path` ends in `.ext`, in any case.
fn has_ext(path: &str, ext: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The built site: every file by its path under the site dir, with the text of each HTML page.
pub(super) struct Tree {
    files: BTreeSet<String>,
    pages: BTreeMap<String, String>,
}

impl Tree {
    /// The built site in `site`, as it is on disk.
    pub(super) fn load(site: &Path) -> Result<Self> {
        let mut tree = Self {
            files: BTreeSet::new(),
            pages: BTreeMap::new(),
        };
        let mut dirs = vec![site.to_path_buf()];
        while let Some(dir) = dirs.pop() {
            for e in std::fs::read_dir(&dir).with_context(|| dir.display().to_string())? {
                let e = e?;
                if e.file_type()?.is_dir() {
                    dirs.push(e.path());
                    continue;
                }
                let rel = e
                    .path()
                    .strip_prefix(site)
                    .context("under the site dir")?
                    .to_string_lossy()
                    .replace('\\', "/");
                if has_ext(&rel, "html") {
                    let text = std::fs::read_to_string(e.path()).with_context(|| rel.clone())?;
                    tree.pages.insert(rel.clone(), text);
                }
                tree.files.insert(rel);
            }
        }
        Ok(tree)
    }

    #[cfg(test)]
    pub(super) fn from(files: &[(&str, &str)]) -> Self {
        Self {
            files: files.iter().map(|f| f.0.to_owned()).collect(),
            pages: files
                .iter()
                .filter(|f| has_ext(f.0, "html"))
                .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
                .collect(),
        }
    }
}

/// Every file in the built site that no generator writes and no other make target owns, and every
/// docs page that is not a document the docs index publishes. `written` is the generators'
/// output, `docs` the site paths the index publishes (`docs/<stem>/`).
pub(super) fn leftovers(tree: &Tree, written: &BTreeSet<String>, docs: &[String]) -> Vec<String> {
    let mut bad = Vec::new();
    for f in &tree.files {
        if !written.contains(f) && !made_elsewhere(f) {
            bad.push(format!(
                "{SITE_DIR}/{f}: no generator writes it (a removed document leaves its page)"
            ));
        }
    }
    let on_disk: BTreeSet<&str> = tree
        .files
        .iter()
        .filter_map(|f| f.strip_prefix("docs/"))
        .filter_map(|f| f.strip_suffix("/index.html"))
        .collect();
    let listed: BTreeSet<&str> = docs
        .iter()
        .filter_map(|d| d.strip_prefix("docs/")?.strip_suffix('/'))
        .collect();
    for d in on_disk.difference(&listed) {
        bad.push(format!(
            "{SITE_DIR}/docs/{d}/: a page for a document the docs index does not publish"
        ));
    }
    for d in listed.difference(&on_disk) {
        bad.push(format!(
            "{SITE_DIR}/docs/{d}/: the docs index publishes it but the page is missing"
        ));
    }
    bad
}

/// Every `attr="value"` of `html`, in order.
fn attrs<'a>(html: &'a str, attr: &str) -> Vec<&'a str> {
    let pat = format!(" {attr}=\"");
    html.split(&pat)
        .skip(1)
        .filter_map(|r| r.split_once('"').map(|v| v.0))
        .collect()
}

/// Every place a page links or loads: `href`, `src` and each URL of a `srcset`.
fn refs(html: &str) -> Vec<&str> {
    let mut out = attrs(html, "href");
    out.extend(attrs(html, "src"));
    for set in attrs(html, "srcset") {
        out.extend(set.split(',').filter_map(|s| s.split_whitespace().next()));
    }
    out
}

/// The ids and names a page can be scrolled to.
fn targets(html: &str) -> BTreeSet<&str> {
    let mut out: BTreeSet<&str> = attrs(html, "id").into_iter().collect();
    out.extend(attrs(html, "name"));
    out
}

/// The built file a site link on `page` names.
fn site_target(page: &str, file: &str) -> String {
    if file.is_empty() {
        return page.to_owned();
    }
    let abs = file.strip_prefix('/').map_or_else(
        || {
            let dir = page.rsplit_once('/').map_or("", |d| d.0);
            normalize(&Path::new(dir).join(file))
        },
        |rooted| normalize(Path::new(rooted)),
    );
    // normalize drops a trailing `/`, so a directory link is read off `file`.
    match (abs.is_empty(), file.ends_with('/')) {
        (true, _) => "index.html".to_owned(),
        (false, true) => format!("{abs}/index.html"),
        (false, false) => abs,
    }
}

/// Every reference of every built page that does not resolve: a site path with no built file,
/// a `#fragment` the target page does not render, and a link into the repository (`repo` with
/// `/blob/main/`, `/tree/main/` or `/raw/main/`) at a path or heading that is not there.
/// `repo_file` reads a repository file, None when it does not exist.
pub(super) fn dead_links(
    tree: &Tree,
    repo: &str,
    repo_file: &dyn Fn(&str) -> Option<String>,
) -> Vec<String> {
    let mut bad = Vec::new();
    let mut gh: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (page, html) in &tree.pages {
        for dest in refs(html) {
            let (file, anchor) = dest.split_once('#').unwrap_or((dest, ""));
            if let Some(rest) = ["blob", "tree", "raw"]
                .iter()
                .find_map(|k| file.strip_prefix(&format!("{repo}/{k}/main/")))
            {
                let path = rest.trim_end_matches('/');
                let Some(text) = repo_file(path) else {
                    bad.push(format!("{page}: {dest}: {path} is not in the repository"));
                    continue;
                };
                if !anchor.is_empty() && has_ext(path, "md") {
                    let ids = gh
                        .entry(path.to_owned())
                        .or_insert_with(|| crate::docs_index::anchors(&text));
                    if !ids.contains(anchor) {
                        bad.push(format!("{page}: {dest}: no heading #{anchor} in {path}"));
                    }
                }
                continue;
            }
            if file.contains("://") || file.starts_with("mailto:") || file.starts_with("data:") {
                continue;
            }
            let target = site_target(page, file);
            // The deploy renders the ignored PNG files after this check runs.
            let deployed = made_elsewhere(&target) && has_ext(&target, "png");
            if !tree.files.contains(&target) && !deployed {
                let hint = if file.starts_with("..") || target.contains(".md") {
                    " (a repository path: link the page, or the file in the repository)"
                } else {
                    ""
                };
                bad.push(format!("{page}: {dest}: no built file {target}{hint}"));
            } else if !anchor.is_empty() {
                let ok = tree
                    .pages
                    .get(&target)
                    .is_some_and(|h| targets(h).contains(anchor));
                if !ok {
                    bad.push(format!("{page}: {dest}: {target} has no #{anchor}"));
                }
            }
        }
    }
    bad
}

/// Every problem of the site built in `site` against what the generators write, with repository
/// links resolved under `root`. Empty when the built site is in step with the docs index and
/// every link of it lands.
pub fn check(root: &Path, site: &Path, outs: &[Out], repo: &str) -> Result<Vec<String>> {
    let tree = Tree::load(site)?;
    // Fail closed: a site with no page has no link that could be reported dead.
    if tree.pages.is_empty() {
        return Ok(vec![format!(
            "{SITE_DIR} has no HTML page: nothing was checked"
        )]);
    }
    let written: BTreeSet<String> = outs
        .iter()
        .filter_map(|o| match o {
            Out::File { path, .. } | Out::Bytes { path, .. } => {
                path.strip_prefix(SITE_DIR)?.strip_prefix('/')
            }
            Out::Block { .. } => None,
        })
        .map(str::to_owned)
        .collect();
    let docs: Vec<String> = on_site()
        .into_iter()
        .map(|(p, ..)| page_path(p.rsplit('/').next().unwrap_or(p).trim_end_matches(".md")))
        .collect();
    let mut bad = leftovers(&tree, &written, &docs);
    let read = |p: &str| -> Option<String> {
        let full = root.join(p);
        if full.is_dir() {
            return Some(String::new());
        }
        std::fs::read(full)
            .ok()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    };
    bad.extend(dead_links(&tree, repo, &read));
    let mut published: BTreeSet<String> = on_site().iter().map(|d| d.0.to_owned()).collect();
    published.extend(crate::docs_index::LEGAL.iter().map(|l| l.0.to_owned()));
    published.insert(crate::site::docs::CHANGELOG.to_owned());
    let facts = guards::Facts {
        repo,
        published,
        docs,
    };
    bad.extend(guards::problems(&tree.pages, &facts));
    Ok(bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "https://github.com/o/r";

    fn page(body: &str) -> String {
        format!(
            "<html><body><main id=\"main\"><h2 id=\"usage\">Usage</h2>{body}</main></body></html>"
        )
    }

    fn repo_file(p: &str) -> Option<String> {
        match p {
            "CONTRIBUTING.md" => Some("# Contributing\n\n## Releasing (maintainers)\n".to_owned()),
            "crates" => Some(String::new()),
            _ => None,
        }
    }

    fn written(tree: &Tree) -> BTreeSet<String> {
        tree.files.clone()
    }

    #[test]
    fn a_clean_tree_has_no_dead_link_or_leftover() {
        let why = page("<a href=\"/docs/cli/#usage\">x</a><a href=\"#usage\">y</a>");
        let cli = page(&format!(
            "<a href=\"{REPO}/blob/main/CONTRIBUTING.md#releasing-maintainers\">c</a>\
             <a href=\"{REPO}/tree/main/crates/\">d</a><img src=\"/assets/a.svg\">\
             <a href=\"https://elsewhere.example/x#y\">e</a>"
        ));
        let tree = Tree::from(&[
            ("docs/why/index.html", &why),
            ("docs/cli/index.html", &cli),
            ("assets/a.svg", ""),
        ]);
        assert!(dead_links(&tree, REPO, &repo_file).is_empty());
        let docs = ["docs/why/".to_owned(), "docs/cli/".to_owned()];
        assert!(leftovers(&tree, &written(&tree), &docs).is_empty());
    }

    #[test]
    fn a_planted_broken_link_dead_anchor_repo_path_or_missing_image_is_reported() {
        let why = page(&format!(
            "<a href=\"/docs/gone/\">broken</a>\
             <a href=\"/docs/cli/#renamed\">dead anchor</a>\
             <a href=\"#nowhere\">dead local anchor</a>\
             <a href=\"../crates/x/src/lib.rs\">repo-relative</a>\
             <img src=\"/assets/missing.webp\" srcset=\"/assets/missing-2x.webp 2x\">\
             <a href=\"{REPO}/blob/main/gone.md\">gone</a>\
             <a href=\"{REPO}/blob/main/CONTRIBUTING.md#releasing\">renamed heading</a>"
        ));
        let cli = page("");
        let tree = Tree::from(&[("docs/why/index.html", &why), ("docs/cli/index.html", &cli)]);
        let bad = dead_links(&tree, REPO, &repo_file).join("\n");
        for want in [
            "/docs/gone/: no built file docs/gone/index.html",
            "/docs/cli/#renamed: docs/cli/index.html has no #renamed",
            "#nowhere: docs/why/index.html has no #nowhere",
            "../crates/x/src/lib.rs: no built file docs/crates/x/src/lib.rs (a repository path",
            "/assets/missing.webp: no built file",
            "/assets/missing-2x.webp: no built file",
            "gone.md is not in the repository",
            "no heading #releasing in CONTRIBUTING.md",
        ] {
            assert!(bad.contains(want), "missing {want:?} in\n{bad}");
        }
        assert_eq!(bad.lines().count(), 8, "{bad}");
    }

    #[test]
    fn a_planted_stale_or_missing_docs_page_is_reported() {
        let tree = Tree::from(&[
            ("docs/why/index.html", ""),
            ("docs/removed/index.html", ""),
            ("assets/og.png", ""),
        ]);
        let mut written = BTreeSet::from(["docs/why/index.html".to_owned()]);
        let docs = ["docs/why/".to_owned(), "docs/cli/".to_owned()];
        let bad = leftovers(&tree, &written, &docs).join("\n");
        assert!(
            bad.contains("target/site/docs/removed/index.html: no generator writes it"),
            "{bad}"
        );
        assert!(
            bad.contains("target/site/docs/removed/: a page for a document"),
            "{bad}"
        );
        assert!(
            bad.contains("target/site/docs/cli/: the docs index publishes it"),
            "{bad}"
        );
        assert_eq!(bad.lines().count(), 3, "{bad}");
        written.insert("docs/removed/index.html".to_owned());
        let docs = ["docs/why/".to_owned(), "docs/removed/".to_owned()];
        assert!(leftovers(&tree, &written, &docs).is_empty());
    }

    /// A fresh build: what `make docs-check` runs, held here too so `cargo test` sees it.
    #[test]
    fn the_built_site_has_no_dead_link_or_leftover_page() {
        let root = crate::repo_root();
        let ws = crate::facts::load(&root).expect("facts");
        let outs = crate::site::render(&root, &ws).expect("render");
        let repo = &ws.packages["gitterrace"].repository;
        let dir = tempfile::tempdir().expect("tempdir");
        let site = crate::site::build(dir.path(), outs).expect("build");
        let outs = crate::site::render(&root, &ws).expect("render");
        let bad = check(&root, &site, &outs, repo).expect("check");
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }
}
