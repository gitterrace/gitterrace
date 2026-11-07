//! `docs/README.md`, the documentation index, and the checks that keep every document honest:
//! each one listed in the index, every relative link and anchor resolving, and every config key
//! and command-line flag a document names existing in the schema or the clap definition, and
//! every repository path in a code span existing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::out::{Out, files_in};
use crate::readme_agents_md::{cli_flags, shown_flags};

const INDEX: &str = "docs/README.md";
const CONFIG: &str = "docs/configuration.md";
/// Flags clap adds to every command, which the command list does not repeat.
const GLOBAL_CLAP: [&str; 2] = ["help", "version"];

/// What a document is for, in the order the index lists them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Start,
    Guide,
    Explain,
    Reference,
    Project,
}

impl Kind {
    const ALL: [Self; 5] = [
        Self::Start,
        Self::Guide,
        Self::Explain,
        Self::Reference,
        Self::Project,
    ];

    const fn heading(self) -> &'static str {
        match self {
            Self::Start => "Start here",
            Self::Guide => "Get one thing done",
            Self::Explain => "Understand how and why",
            Self::Reference => "Look something up",
            Self::Project => "The project",
        }
    }
}

/// Every document, with what it covers. A Markdown file in the root or under `docs/` that is
/// not here, or an entry whose file is gone, fails `check`.
const DOCS: &[(&str, Kind, &str)] = &[
    (
        "README.md",
        Kind::Start,
        "Install, the first run, every command",
    ),
    ("docs/why.md", Kind::Start, "What gitterrace is for"),
    (
        "docs/actions.md",
        Kind::Guide,
        "Send a selection to another app, or run your own program on it",
    ),
    (
        "docs/online.md",
        Kind::Guide,
        "Publish a static export, public mode, a reverse proxy",
    ),
    (
        "docs/architecture.md",
        Kind::Explain,
        "Crates, the decisions behind them, the cache, routes, highlighted languages",
    ),
    (
        "docs/security.md",
        Kind::Explain,
        "What is protected, at which door, and the test that holds each door shut",
    ),
    (
        "docs/design/streaming-insights.md",
        Kind::Explain,
        "The insights store on disk and how pages read it in some MB",
    ),
    (
        "docs/internals/counting.md",
        Kind::Explain,
        "How every insights number is counted: one membership rule and one local-day rule",
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
    (
        "docs/benchmarks.md",
        Kind::Reference,
        "What is measured, the budgets and the newest results",
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
    let listed: BTreeSet<&str> = DOCS.iter().map(|d| d.0).collect();
    let mut bad = Vec::new();
    for d in &docs {
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
    let mut texts = BTreeMap::new();
    for d in &docs {
        let text = std::fs::read_to_string(root.join(d)).with_context(|| d.clone())?;
        texts.insert(d.clone(), text);
    }
    let keys = Keys::load();
    for (path, text) in &texts {
        bad.extend(check_links(root, path, text, &texts));
        bad.extend(keys.check(path, text));
        bad.extend(check_paths(root, path, text));
    }
    bad.extend(keys.reverse(&texts));
    Ok(bad)
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
fn anchors(text: &str) -> BTreeSet<String> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = BTreeSet::new();
    let mut current: Option<String> = None;
    for ev in Parser::new_ext(text, Options::all()) {
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
    for ev in Parser::new_ext(text, Options::all()) {
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
const PATH_ROOTS: [&str; 5] = ["crates/", "scripts/", "xtask/", "bench/", "docs/"];

fn check_paths(root: &Path, path: &str, text: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for ev in Parser::new_ext(text, Options::all()) {
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
fn normalize(p: &Path) -> String {
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

/// The names a document may use: config keys from the schema and long flags from clap.
struct Keys {
    sections: BTreeSet<&'static str>,
    ids: BTreeSet<&'static str>,
    flags: BTreeSet<String>,
    /// [`Self::flags`] without the hidden ones, the flags the README must list.
    shown: BTreeSet<String>,
}

impl Keys {
    fn load() -> Self {
        let keys = gitterrace_config::keys();
        Self {
            sections: keys.iter().map(|k| k.section).collect(),
            ids: keys.iter().map(|k| k.id).collect(),
            flags: cli_flags(),
            shown: shown_flags(),
        }
    }

    /// Names in inline code (`server.addr`, `--public`, `git terrace export --commits`) and in
    /// fenced `toml` blocks under a config section.
    fn check(&self, path: &str, text: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let mut toml = false;
        for ev in Parser::new_ext(text, Options::all()) {
            match ev {
                Event::Code(c) => bad.extend(self.span(path, &c)),
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                    toml = lang.as_ref() == "toml";
                }
                Event::End(TagEnd::CodeBlock) => toml = false,
                Event::Text(t) if toml => bad.extend(self.toml(path, &t)),
                Event::Text(t) => {
                    for line in t
                        .lines()
                        .filter(|l| l.trim_start().starts_with("git terrace"))
                    {
                        bad.extend(self.flags_in(path, line));
                    }
                }
                _ => {}
            }
        }
        bad
    }

    fn span(&self, path: &str, code: &str) -> Vec<String> {
        let code = code.trim();
        if code.starts_with("git terrace") || (code.starts_with("--") && !code.contains(' ')) {
            return self.flags_in(path, code);
        }
        let code = code.split_whitespace().next().unwrap_or(code);
        let Some((section, name)) = code.split_once('.') else {
            return Vec::new();
        };
        let is_file = ["html", "md", "json", "toml", "rs", "css", "js"].contains(&name);
        let is_key = self.sections.contains(section)
            && !is_file
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_lowercase() || c == '_');
        if is_key && !self.ids.contains(code) {
            return vec![format!("{path}: `{code}` is not a config key")];
        }
        Vec::new()
    }

    fn flags_in(&self, path: &str, text: &str) -> Vec<String> {
        text.split_whitespace()
            .filter_map(|w| w.strip_prefix("--"))
            .map(|w| w.split(['=', ',', ')', ']', '`']).next().unwrap_or(w))
            .filter(|f| !f.is_empty() && !f.ends_with('-') && !f.contains('<'))
            .filter(|f| !self.flags.contains(*f))
            .map(|f| format!("{path}: `--{f}` is not a flag of git terrace"))
            .collect()
    }

    /// The other direction: every key has its row in the configuration page and every flag
    /// its place in the README's command list, so neither reference leaves one out.
    fn reverse(&self, texts: &BTreeMap<String, String>) -> Vec<String> {
        let mut bad = Vec::new();
        if let Some(config) = texts.get(CONFIG) {
            for id in &self.ids {
                let (section, name) = id.split_once('.').unwrap_or((id, ""));
                let row = format!("| `{name}` |");
                let head = format!("## `[{section}]`");
                let found = config
                    .split("\n## ")
                    .any(|part| format!("## {part}").starts_with(&head) && part.contains(&row));
                if !found {
                    bad.push(format!("{CONFIG}: no row for `{id}`"));
                }
            }
        }
        if let Some(readme) = texts.get("README.md") {
            for f in self
                .shown
                .iter()
                .filter(|f| !GLOBAL_CLAP.contains(&f.as_str()))
            {
                if !readme.contains(&format!("--{f}")) {
                    bad.push(format!("README.md: the command list leaves out `--{f}`"));
                }
            }
        }
        bad
    }

    fn toml(&self, path: &str, text: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let mut section: Option<&str> = None;
        for line in text.lines().map(str::trim) {
            if let Some(h) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = self.sections.get(h).copied();
                continue;
            }
            let (Some(s), Some((name, _))) = (section, line.split_once('=')) else {
                continue;
            };
            let id = format!("{s}.{}", name.trim());
            if !self.ids.contains(id.as_str()) {
                bad.push(format!(
                    "{path}: `{id}` in a toml example is not a config key"
                ));
            }
        }
        bad
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
