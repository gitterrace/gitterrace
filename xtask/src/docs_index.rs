//! `docs/README.md`, the documentation index, and the checks that keep every document honest:
//! each one listed in the index, every relative link and anchor resolving, and every config key
//! and command-line flag a document names existing in the schema or the clap definition, and
//! every repository path in a code span existing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::out::{Out, files_in};

mod docker;
mod keys;
use keys::Keys;

const INDEX: &str = "docs/README.md";

/// What a document is for, in the order the index lists them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Start,
    Guide,
    Reference,
    Explain,
    Contribute,
    Legal,
    Project,
}

impl Kind {
    const ALL: [Self; 7] = [
        Self::Start,
        Self::Guide,
        Self::Reference,
        Self::Explain,
        Self::Contribute,
        Self::Legal,
        Self::Project,
    ];

    const fn heading(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Guide => "Use",
            Self::Reference => "Look up",
            Self::Explain => "Background",
            Self::Contribute => "Contribute",
            Self::Legal => "Legal",
            Self::Project => "The project",
        }
    }
}

/// Every document, with what it covers. A Markdown file in the root or under `docs/` that is
/// not here, or an entry whose file is gone, fails `check`.
const DOCS: &[(&str, Kind, &str)] = &[
    (
        "docs/getting-started.md",
        Kind::Start,
        "Install, the first run, Windows, upgrading and troubleshooting",
    ),
    (
        "README.md",
        Kind::Start,
        "The repository's front page: install, quickstart and every command",
    ),
    (
        "docs/using.md",
        Kind::Guide,
        "Repositories, the log graph, outlines, keys, settings",
    ),
    (
        "docs/actions.md",
        Kind::Guide,
        "Send a selection to another app, or run your own program on it",
    ),
    (
        "docs/online.md",
        Kind::Guide,
        "Publish a static export, public mode, Docker, a reverse proxy",
    ),
    (
        "docs/configuration.md",
        Kind::Reference,
        "Every `config.toml` key, its default and the naming rule",
    ),
    (
        "docs/cli.md",
        Kind::Reference,
        "Every command and flag of `git terrace`",
    ),
    (
        "docs/api.md",
        Kind::Reference,
        "The JSON API and the MCP tools",
    ),
    ("docs/why.md", Kind::Explain, "What gitterrace is for"),
    (
        "docs/security.md",
        Kind::Explain,
        "What is protected, from whom, and what holds each door shut",
    ),
    (
        "docs/benchmarks.md",
        Kind::Explain,
        "What is measured, the budgets and the newest results",
    ),
    (
        "docs/architecture.md",
        Kind::Contribute,
        "Crates, decisions, the cache, routes, security guards and their tests",
    ),
    (
        "docs/legal/imprint.md",
        Kind::Legal,
        "Who runs the site, as the law requires",
    ),
    (
        "docs/legal/privacy.md",
        Kind::Legal,
        "What the site does with visitors' data",
    ),
    (
        "CONTRIBUTING.md",
        Kind::Project,
        "Setup, the checks, commits, releasing",
    ),
    (
        "AGENTS.md",
        Kind::Project,
        "Crate layout and code conventions",
    ),
    (
        "SECURITY.md",
        Kind::Project,
        "Reporting a vulnerability, and what is in scope",
    ),
    (
        "CHANGELOG.md",
        Kind::Project,
        "Changes per release, written from the commits",
    ),
];

/// The documents the site publishes: every kind but the project's own files, as repo path,
/// the heading of its group and what it covers, in index order. The README is the landing
/// page's own source and has raw HTML for its header, so it is not a page of its own.
pub fn on_site() -> Vec<(&'static str, &'static str, &'static str)> {
    Kind::ALL
        .iter()
        .filter(|k| !matches!(**k, Kind::Project | Kind::Legal))
        .flat_map(|k| {
            DOCS.iter()
                .filter(move |d| d.1 == *k && d.0 != "README.md")
                .map(move |d| (d.0, k.heading(), d.2))
        })
        .collect()
}

/// The legal documents, as repo path and the label the footer shows. They are pages of their
/// own under `/legal/`, not part of the docs index.
pub const LEGAL: [(&str, &str); 2] = [
    ("docs/legal/imprint.md", "Imprint"),
    ("docs/legal/privacy.md", "Privacy"),
];

pub fn render() -> Out {
    let mut s = String::new();
    for kind in Kind::ALL {
        let _ = write!(
            s,
            "## {}\n\n| Document | Covers |\n| --- | --- |\n",
            kind.heading()
        );
        for (path, _, what) in DOCS.iter().filter(|d| d.1 == kind) {
            let _ = writeln!(s, "| [{}]({}) | {what} |", title(path), relative(path));
        }
        s.push('\n');
    }
    Out::block(INDEX, "doc-index", s)
}

/// What the document at `path` covers, its index line; None for a path the index lacks.
pub fn covers(path: &str) -> Option<&'static str> {
    DOCS.iter().find(|d| d.0 == path).map(|d| d.2)
}

fn title(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.strip_suffix(".md").unwrap_or(name)
}

/// The link from `docs/README.md` to `path`.
fn relative(path: &str) -> String {
    path.strip_prefix("docs/")
        .map_or_else(|| format!("../{path}"), str::to_owned)
}

/// Every problem the documents have: unlisted documents, broken links or anchors, unknown keys
/// or flags. Empty when all is well.
pub fn check(root: &Path) -> Result<Vec<String>> {
    let docs = documents(root)?;
    let mut bad = registry_problems(&docs, DOCS.iter().map(|d| d.0));
    let mut texts = BTreeMap::new();
    for d in &docs {
        let text = std::fs::read_to_string(root.join(d)).with_context(|| d.clone())?;
        texts.insert(d.clone(), text);
    }
    let keys = Keys::load();
    let (port, docker_bad) = docker::port(root);
    bad.extend(docker_bad);
    for (path, text) in &texts {
        if let Some(port) = port {
            bad.extend(docker::check(path, text, port));
        }
        bad.extend(check_links(root, path, text, &texts));
        bad.extend(keys.check(path, text));
        bad.extend(keys.defaults(path, text));
        bad.extend(check_paths(root, path, text));
    }
    bad.extend(keys.reverse(&texts));
    Ok(bad)
}

/// Both directions of the registry: a document on disk that is not listed, and a listed
/// document that is not on disk.
fn registry_problems<'a>(docs: &[String], listed: impl Iterator<Item = &'a str>) -> Vec<String> {
    let listed: BTreeSet<&str> = listed.collect();
    let mut bad = Vec::new();
    for d in docs {
        if d != INDEX && !listed.contains(d.as_str()) {
            bad.push(format!(
                "{d}: not listed in {INDEX} (add it to DOCS in xtask)"
            ));
        }
    }
    for l in &listed {
        if !docs.iter().any(|d| d == l) {
            bad.push(format!("{INDEX}: lists {l}, which does not exist"));
        }
    }
    bad
}

/// The Markdown files of the root and of `docs/` (with its subfolders), as repo paths.
fn documents(root: &Path) -> Result<Vec<String>> {
    let mut out: Vec<String> = files_in(root, "md")?;
    let mut dirs = vec!["docs".to_owned()];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(root.join(&dir)).with_context(|| dir.clone())? {
            let e = e?;
            let name = format!("{dir}/{}", e.file_name().to_string_lossy());
            if e.file_type()?.is_dir() {
                dirs.push(name);
            } else if is_md(&name) {
                out.push(name);
            }
        }
    }
    out.sort();
    Ok(out)
}

fn is_md(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// Heading anchors as the page renderer and most Markdown hosts make them.
pub fn anchors(text: &str) -> BTreeSet<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = BTreeSet::new();
    let mut current: Option<String> = None;
    for ev in Parser::new_ext(text, gitterrace_web::markdown_options()) {
        match ev {
            Event::Start(Tag::Heading { .. }) => current = Some(String::new()),
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = current.as_mut() {
                    c.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(h) = current.take() {
                    let base = slug(&h);
                    let n = seen.entry(base.clone()).or_insert(0);
                    out.insert(if *n == 0 { base } else { format!("{base}-{n}") });
                    *n += 1;
                }
            }
            _ => {}
        }
    }
    out
}

fn slug(text: &str) -> String {
    text.trim()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c == '-' || c == '_' || c.is_alphanumeric() => Some(c),
            _ => None,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

fn check_links(
    root: &Path,
    path: &str,
    text: &str,
    texts: &BTreeMap<String, String>,
) -> Vec<String> {
    let dir = Path::new(path).parent().unwrap_or_else(|| Path::new(""));
    let mut bad = Vec::new();
    for ev in Parser::new_ext(text, gitterrace_web::markdown_options()) {
        let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = ev else {
            continue;
        };
        let dest = dest_url.to_string();
        if dest.contains("://") || dest.starts_with("mailto:") {
            continue;
        }
        let (file, anchor) = dest.split_once('#').unwrap_or((&dest, ""));
        let target = if file.is_empty() {
            path.to_owned()
        } else {
            normalize(&dir.join(file))
        };
        if !file.is_empty() && !root.join(&target).exists() {
            bad.push(format!("{path}: link to {dest}: {target} does not exist"));
            continue;
        }
        if anchor.is_empty() || !is_md(&target) {
            continue;
        }
        let found = texts.get(&target).map_or_else(
            || {
                std::fs::read_to_string(root.join(&target))
                    .is_ok_and(|t| anchors(&t).contains(anchor))
            },
            |t| anchors(t).contains(anchor),
        );
        if !found {
            bad.push(format!(
                "{path}: link to {dest}: no heading #{anchor} in {target}"
            ));
        }
    }
    bad
}

/// Folders whose paths a document may name in a code span. A span that starts with one and
/// holds no placeholder must name a file or folder that exists, so a moved script or crate
/// cannot leave a dead pointer behind.
const PATH_ROOTS: [&str; 5] = ["crates/", "scripts/", "xtask/", "tests/", "docs/"];

fn check_paths(root: &Path, path: &str, text: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for ev in Parser::new_ext(text, gitterrace_web::markdown_options()) {
        let Event::Code(code) = ev else { continue };
        let span = code.trim_end_matches('/');
        let plain = !span.contains(|c: char| c.is_whitespace() || "*<>{}[]".contains(c));
        if plain && PATH_ROOTS.iter().any(|r| span.starts_with(r)) && !root.join(span).exists() {
            bad.push(format!("{path}: `{span}` does not exist"));
        }
    }
    bad
}

/// `a/b/../c` as `a/c`, with `/` separators.
pub fn normalize(p: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                parts.pop();
            }
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The registry is checked both ways: a file nobody listed and a listing nobody wrote.
    #[test]
    fn a_listed_document_missing_on_disk_and_an_unlisted_one_both_fail() {
        let on_disk = vec!["docs/a.md".to_owned(), "docs/new.md".to_owned()];
        let bad = registry_problems(&on_disk, ["docs/a.md", "docs/gone.md"].into_iter());
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(bad[0].contains("docs/new.md: not listed"), "{bad:?}");
        assert!(
            bad[1].contains("lists docs/gone.md, which does not exist"),
            "{bad:?}"
        );
        assert!(registry_problems(&on_disk, ["docs/a.md", "docs/new.md"].into_iter()).is_empty());
    }

    /// The index and the checks read the same list: an unlisted page, a dead link, a missing
    /// anchor, an unknown key and an unknown flag each go red.
    #[test]
    fn each_kind_of_drift_goes_red() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs")).expect("docs");
        for (p, ..) in DOCS {
            let file = root.join(p);
            std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
            std::fs::write(&file, "# Title\n\n## Part two\n").expect("write");
        }
        std::fs::write(root.join(INDEX), "# Docs\n").expect("index");
        let reverse = |l: &String| l.contains("no row for") || l.contains("leaves out");
        let clean = check(root).expect("check");
        assert!(clean.iter().all(reverse), "{clean:?}");
        assert!(
            clean.iter().any(|l| l.contains("no row for `server.addr`")),
            "{clean:?}"
        );
        assert!(
            clean.iter().any(|l| l.contains("leaves out `--public`")),
            "{clean:?}"
        );

        std::fs::write(root.join("docs/stray.md"), "# Stray\n").expect("stray");
        std::fs::write(
            root.join("docs/why.md"),
            "# Why\n\n[a](gone.md) [b](api.md#part-two) [c](api.md#part-3) [d](#why)\n\n\
             Set `server.public = true`, `server.addr`, `index.html`, `--public`, `--bogus`.\n\n\
             ```toml\n[server]\nis_public = true\nlive = 3\n```\n\n\
             ```sh\ngit terrace export --commits 3 --pages 9\n```\n\n\
             See `scripts/gone.sh` and `scripts/` and `docs/why.md`.\n",
        )
        .expect("why");
        let bad: Vec<String> = check(root)
            .expect("check")
            .into_iter()
            .filter(|l| !reverse(l))
            .collect();
        let bad = bad.join("\n");
        for needle in [
            "docs/stray.md: not listed",
            "gone.md does not exist",
            "no heading #part-3",
            "`server.public` is not a config key",
            "`--bogus` is not a flag",
            "`server.live` in a toml example",
            "`--pages` is not a flag",
            "`scripts/gone.sh` does not exist",
        ] {
            assert!(bad.contains(needle), "missing {needle:?} in:\n{bad}");
        }
        for fine in [
            "part-two",
            "#why",
            "server.addr",
            "index.html",
            "--public",
            "--commits",
            "`docs/why.md` does not",
        ] {
            assert!(!bad.contains(fine), "{fine} flagged:\n{bad}");
        }
    }

    #[test]
    fn anchors_follow_the_page_renderer() {
        let a = anchors("# A `[server]` key\n\n## Same\n\n## Same\n\n### 2.1 Byte layout\n");
        for want in ["a-server-key", "same", "same-1", "21-byte-layout"] {
            assert!(a.contains(want), "{want} in {a:?}");
        }
    }
}
