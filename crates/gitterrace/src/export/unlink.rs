//! Links an export cannot serve. The copy holds a scoped set of pages, and a static host
//! ignores a query string, so a link outside that set or one that asks a page for a filter
//! would land on a 404 or on the unfiltered page. Once every page is on disk, each written
//! page is read back and such a link loses its target and stays as its text: an `<a>` without
//! `href` is not a link. Checking the files themselves, not the scope, also covers a page in
//! scope that the server did not answer or whose name was refused.

use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use gitterrace_git::percent_decode;

/// Drops every dead link from every HTML page under `out`; returns how many it dropped.
pub(super) fn unlink_dead(out: &Path) -> anyhow::Result<usize> {
    let mut dropped = 0;
    let mut todo = vec![out.to_path_buf()];
    while let Some(dir) = todo.pop() {
        for e in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let p = e?.path();
            let kind = std::fs::symlink_metadata(&p)?.file_type();
            if kind.is_dir() {
                todo.push(p);
            } else if kind.is_file() && p.extension().is_some_and(|x| x == "html") {
                let html = std::fs::read_to_string(&p)
                    .with_context(|| format!("reading {}", p.display()))?;
                let base = p.parent().unwrap_or(out).to_path_buf();
                let (text, n) = unlink(&html, |link| written(out, &base, link));
                if n > 0 {
                    dropped += n;
                    crate::export::write::write_staged(&p, text.as_bytes())?;
                }
            }
        }
    }
    Ok(dropped)
}

/// `html` with every relative `href`/`src` that `ok` refuses removed, and how many went.
pub(super) fn unlink(html: &str, ok: impl Fn(&str) -> bool) -> (String, usize) {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut n = 0;
    while let Some((at, marker)) = next_attr(rest) {
        let value = at + marker.len();
        let end = rest[value..].find('"').map_or(rest.len(), |e| value + e);
        // Only a whole attribute, never the tail of `data-href`.
        let whole = at > 0 && rest.as_bytes()[at - 1] == b' ';
        if whole && !ok(&rest[value..end]) {
            out.push_str(&rest[..at - 1]);
            rest = rest.get(end + 1..).unwrap_or("");
            n += 1;
        } else {
            out.push_str(&rest[..end]);
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    (out, n)
}

fn next_attr(s: &str) -> Option<(usize, &'static str)> {
    ["href=\"", "src=\""]
        .into_iter()
        .filter_map(|m| s.find(m).map(|i| (i, m)))
        .min_by_key(|(i, _)| *i)
}

/// Whether `link`, written relative to the folder `base`, reaches a file under `out` that
/// serves it. A page (`index.html`) cannot read a query; any other file is the same with one
/// (assets carry a `?v=` cache key). Links off the site, fragments and `data:` pass.
pub(super) fn written(out: &Path, base: &Path, link: &str) -> bool {
    let link = link.replace("&amp;", "&");
    if link.is_empty() || link.starts_with('#') || link.contains(':') || link.starts_with("//") {
        return true;
    }
    let cut = link.find(['?', '#']).unwrap_or(link.len());
    let (path, tail) = link.split_at(cut);
    let Some(file) = resolve(out, base, &percent_decode(path)) else {
        return false;
    };
    if tail.starts_with('?') && file.file_name().is_some_and(|f| f == "index.html") {
        return false;
    }
    file.is_file()
}

/// `rel` joined to `base`, refused when it climbs out of `out`.
fn resolve(out: &Path, base: &Path, rel: &str) -> Option<PathBuf> {
    let mut p = base.to_path_buf();
    for c in Path::new(rel).components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !p.pop() {
                    return None;
                }
            }
            Component::Normal(s) => p.push(s),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    p.starts_with(out).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::{unlink, written};

    #[test]
    fn links_outside_the_export_and_page_queries_lose_their_target() {
        let html = concat!(
            r#"<a href="../log/HEAD/index.html">in</a>"#,
            r#"<a class="x" href="../log/side/index.html">out</a>"#,
            r##"<a href="../log/HEAD/index.html?q=a&amp;b">q</a><a href="#top">frag</a>"##,
            r#"<a data-href="gone">d</a><a href="https://x.dev/">e</a>"#
        );
        let (out, n) = unlink(html, |l| {
            l == "../log/HEAD/index.html" || l.contains(':') || l == "#top"
        });
        assert_eq!(
            out,
            concat!(
                r#"<a href="../log/HEAD/index.html">in</a><a class="x">out</a>"#,
                r##"<a>q</a><a href="#top">frag</a>"##,
                r#"<a data-href="gone">d</a><a href="https://x.dev/">e</a>"#
            )
        );
        assert_eq!(n, 2);
    }

    #[test]
    fn a_link_resolves_only_to_a_file_inside_the_export() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path();
        std::fs::create_dir_all(out.join("refs")).expect("mkdir");
        std::fs::create_dir_all(out.join("static")).expect("mkdir");
        std::fs::write(out.join("refs/index.html"), "x").expect("write");
        std::fs::write(out.join("static/app.css"), "x").expect("write");
        let base = out.join("refs");
        assert!(written(out, &base, "./index.html#top"));
        assert!(written(out, &base, "../static/app.css?v=1"));
        assert!(!written(out, &base, "./index.html?q=x"));
        assert!(!written(out, &base, "../log/HEAD/index.html"));
        assert!(!written(out, &base, "../../index.html"));
    }
}
