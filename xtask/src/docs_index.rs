//! `docs/README.md`, the documentation index: every document, grouped by what a reader needs.

use std::fmt::Write as _;

use crate::out::Out;

const INDEX: &str = "docs/README.md";

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

/// Every document, with what it covers.
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
        "docs/design/streaming-insights.md",
        Kind::Explain,
        "The insights store on disk and how pages read it in some MB",
    ),
    (
        "docs/configuration.md",
        Kind::Reference,
        "Every `config.toml` key, its default and the naming rule",
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
