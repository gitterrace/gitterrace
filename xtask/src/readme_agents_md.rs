//! The generated blocks of `README.md`, `AGENTS.md` and `docs/getting-started.md`, and the
//! crate and cache facts `docs/architecture.md` shows. Everything here is read from the
//! workspace or from the registries the code itself uses.

use std::fmt::Write as _;

use clap::CommandFactory as _;
use gitterrace_actions::schema::{KINDS, kind_name};

use crate::action_scripts::Script;
use crate::facts::Workspace;
use crate::out::Out;

#[path = "../../crates/gitterrace/src/cli.rs"]
#[expect(
    dead_code,
    reason = "xtask only reads the definition, never the parsed values"
)]
pub mod cli;

const README: &str = "README.md";
/// The docs page the README's install and quickstart blocks are shared with.
const START: &str = "docs/getting-started.md";

/// The first run, the same in the README, on the getting started page and on the landing page.
pub const QUICKSTART: &str = "cd path/to/any/git/repo
git terrace          # serves on http://127.0.0.1:4242, open it in your browser
cd ../another/repo
git terrace          # joins the server already running, no second port
";

pub fn render(ws: &Workspace, scripts: &[Script]) -> Vec<Out> {
    let bin = ws.packages.get("gitterrace");
    let install = install_md(ws);
    let msrv = bin.map_or_else(String::new, |p| {
        format!(
            "Building with cargo needs Rust {} or newer.",
            p.rust_version
        )
    });

    let kinds: Vec<String> = KINDS
        .iter()
        .map(|(k, _)| format!("`{}`", kind_name(*k)))
        .collect();
    let files: Vec<String> = scripts.iter().map(|s| format!("`{}`", s.file)).collect();
    let actions = format!(
        "Run your own program on a selection ({}). Examples in `docs/actions/`: {}.\n",
        kinds.join(", "),
        files.join(", ")
    );

    vec![
        Out::block(
            README,
            "pitch",
            bin.map_or_else(String::new, |p| p.description.clone()),
        ),
        Out::block(README, "msrv", msrv.clone()),
        Out::block(README, "install", install.clone()),
        Out::block(README, "quickstart", quickstart_md()),
        Out::block(START, "msrv", msrv),
        Out::block(START, "install", install),
        Out::block(START, "quickstart", quickstart_md()),
        Out::block(README, "commands", commands()),
        Out::block(README, "actions", actions),
        Out::block(README, "license", license_md(bin)),
        Out::block(README, "demo", demo_md(ws)),
        Out::block(README, "shots", shots_md()),
        Out::block("AGENTS.md", "crates", crate_table(ws)),
    ]
}

/// The live demo link and what it shows, from `[workspace.metadata.demo]`; nothing when there
/// is no demo.
pub fn demo_md(ws: &Workspace) -> String {
    if ws.demo.is_empty() {
        return String::new();
    }
    let names: Vec<String> = ws
        .demo_repos
        .iter()
        .map(|(name, tag)| {
            // jq tags as `jq-1.8.2`: show `jq 1.8.2`, not the name twice.
            let v = tag.strip_prefix(&format!("{name}-")).unwrap_or(tag);
            format!("{name} {v}")
        })
        .collect();
    let shows = match names.as_slice() {
        [] => String::new(),
        [one] => format!(": {one}, exported"),
        [rest @ .., last] => format!(": {} and {last}, exported", rest.join(", ")),
    };
    format!(
        "Try it without installing: [the live demo]({}){shows}.\n\n\
         The screenshots show the same projects: [jq](https://github.com/jqlang/jq) and \
         [ripgrep](https://github.com/BurntSushi/ripgrep), both MIT licensed. Thanks to their \
         authors.\n",
        ws.demo
    )
}

/// [`QUICKSTART`] as a fenced block.
fn quickstart_md() -> String {
    format!("```bash\n{QUICKSTART}```\n")
}

/// One screenshot, light and dark, from the same list the site uses.
fn shots_md() -> String {
    let (name, alt, _) = crate::site::SHOTS[0];
    format!(
        "<picture>\n<source media=\"(prefers-color-scheme: \
        dark)\" srcset=\"docs/screenshots/{name}-dark.png\">\n<img \
        src=\"docs/screenshots/{name}-light.png\" alt=\"{alt}\">\n</picture>\n"
    )
}

/// The license line, from the binary's manifest; the full text is the root `LICENSE` file.
fn license_md(bin: Option<&crate::facts::Package>) -> String {
    bin.map_or_else(String::new, |p| {
        format!("{}, see [LICENSE](LICENSE).", p.license)
    })
}

/// The shell installer dist attaches to every release (`installers = ["shell"]`), named
/// `<package>-installer.sh`. The site redirects `/install.sh` here.
pub fn installer_url(repo: &str, name: &str) -> String {
    format!("{repo}/releases/latest/download/{name}-installer.sh")
}

/// The one-line install, served by the site's `/install.sh` redirect.
pub fn curl_install(home: &str) -> String {
    format!("curl -fsSL {}/install.sh | sh", home.trim_end_matches('/'))
}

/// The install block: the README and the site both show exactly this.
pub fn install_md(ws: &Workspace) -> String {
    let mut install = String::new();
    if let Some(p) = ws.packages.get("gitterrace") {
        install.push_str("```bash\n");
        for (_, line) in install_lines(ws) {
            let _ = writeln!(install, "{line}");
        }
        install.push_str("```\n\n");
        let names: Vec<String> = p.bins.iter().map(|b| format!("`{b}`")).collect();
        let _ = writeln!(
            install,
            "Installs {}; git runs it as `git terrace`.",
            names.join(" and ")
        );
    }
    install
}

/// The install commands as (way, command): curl, brew and cargo, each only when its source
/// exists. The README and the site both read this, so they cannot disagree.
pub fn install_lines(ws: &Workspace) -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    let Some(p) = ws.packages.get("gitterrace") else {
        return lines;
    };
    if !p.homepage.is_empty() {
        lines.push(("curl", curl_install(&p.homepage)));
    }
    // `owner/homebrew-tap` installs as `owner/tap/<formula>`.
    if let Some(tap) = ws
        .tap
        .strip_suffix("/homebrew-tap")
        .map(|o| format!("{o}/tap"))
    {
        lines.push(("brew", format!("brew install {tap}/{}", p.name)));
    }
    lines.push((
        "cargo",
        format!("cargo install --locked --git {} {}", p.repository, p.name),
    ));
    lines
}

/// One line per subcommand and flag, from the clap definition.
fn commands() -> String {
    let rows = commands_rows();
    let width = rows.iter().map(|(a, _)| a.len()).max().unwrap_or(0);
    let mut s = String::from("```text\n");
    for (a, b) in rows {
        let _ = writeln!(s, "{a:width$}  {b}");
    }
    s.push_str("```\n");
    s
}

/// `(usage, about)` of the root command and each subcommand.
pub fn commands_rows() -> Vec<(String, String)> {
    let cmd = cli::Cli::command();
    let flags = |c: &clap::Command| -> String {
        let f: Vec<String> = c
            .get_arguments()
            .filter(|a| {
                a.get_long().is_some()
                    && !a.is_hide_set()
                    && a.get_id() != "help"
                    && a.get_id() != "version"
            })
            .map(|a| format!("--{}", a.get_long().unwrap_or_default()))
            .collect();
        if f.is_empty() {
            String::new()
        } else {
            format!(" [{}]", f.join(" "))
        }
    };
    let positionals = |c: &clap::Command| -> String {
        c.get_positionals()
            .map(|a| {
                let n = a.get_id().as_str().replace('_', "-");
                if a.is_required_set() {
                    format!(" <{n}>")
                } else {
                    format!(" [{n}]")
                }
            })
            .collect()
    };
    let mut rows: Vec<(String, String)> = vec![(
        format!("git terrace{}", positionals(&cmd)),
        format!("{}{}", first_line(&cmd), flags(&cmd)),
    )];
    for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
        let own = |c: &clap::Command| -> String {
            // Global flags are listed once, on the first row.
            let f: Vec<String> = c
                .get_arguments()
                .filter(|a| a.get_long().is_some() && !a.is_global_set() && a.get_id() != "help")
                .map(|a| format!("--{}", a.get_long().unwrap_or_default()))
                .collect();
            if f.is_empty() {
                String::new()
            } else {
                format!(" [{}]", f.join(" "))
            }
        };
        rows.push((
            format!(
                "git terrace {}{}",
                sub.get_name(),
                positionals_no_global(sub)
            ),
            format!("{}{}", first_line(sub), own(sub)),
        ));
    }
    rows
}

/// Every documented long flag of the root command and its subcommands, from the clap definition.
/// Hidden (retired) flags are left out: a document naming one is stale, and none has to list it.
pub fn cli_flags() -> std::collections::BTreeSet<String> {
    let cmd = cli::Cli::command();
    let mut out = std::collections::BTreeSet::new();
    for c in std::iter::once(&cmd).chain(cmd.get_subcommands()) {
        out.extend(
            c.get_arguments()
                .filter(|a| !a.is_hide_set())
                .filter_map(|a| a.get_long().map(str::to_owned)),
        );
    }
    out.extend(["help".to_owned(), "version".to_owned()]);
    out
}

/// The [`cli_flags`] a user is shown: a hidden flag (kept so old scripts start) is accepted
/// in the docs but not owed a line in the README command list.
pub fn shown_flags() -> std::collections::BTreeSet<String> {
    let cmd = cli::Cli::command();
    let hidden: std::collections::BTreeSet<String> = std::iter::once(&cmd)
        .chain(cmd.get_subcommands())
        .flat_map(clap::Command::get_arguments)
        .filter(|a| a.is_hide_set())
        .filter_map(|a| a.get_long().map(str::to_owned))
        .collect();
    cli_flags()
        .into_iter()
        .filter(|f| !hidden.contains(f))
        .collect()
}

fn positionals_no_global(c: &clap::Command) -> String {
    c.get_positionals()
        .filter(|a| !a.is_global_set())
        .map(|a| {
            let n = a.get_id().as_str().replace('_', "-");
            if a.is_required_set() {
                format!(" <{n}>")
            } else {
                format!(" [{n}]")
            }
        })
        .collect()
}

fn first_line(c: &clap::Command) -> String {
    c.get_about()
        .map(ToString::to_string)
        .unwrap_or_default()
        .replace('\n', " ")
}

fn crate_table(ws: &Workspace) -> String {
    let mut s = String::from("| Crate | Owns |\n| --- | --- |\n");
    let order = [
        "gitterrace-types",
        "gitterrace-git",
        "gitterrace-index",
        "gitterrace-config",
        "gitterrace-actions",
        "gitterrace-api",
        "gitterrace-web",
        "gitterrace",
    ];
    let mut names: Vec<&str> = ws.crates().map(|p| p.name.as_str()).collect();
    names.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(order.len()));
    for n in names {
        let p = &ws.packages[n];
        let extra = match p.bins.as_slice() {
            [] => String::new(),
            [one] => format!(" Binary `{one}`."),
            many => format!(" Binaries `{}`.", many.join("` and `")),
        };
        let sep = if extra.is_empty() || p.description.ends_with('.') {
            ""
        } else {
            "."
        };
        let _ = writeln!(s, "| `{}` | {}{sep}{extra} |", p.name, p.description);
    }
    s
}

/// One short label per crate for the diagram, so a box says what it is.
const ROLES: [(&str, &str); 9] = [
    ("gitterrace", "the binary and CLI"),
    ("web", "server, pages, guard"),
    ("api", "JSON API and MCP"),
    ("actions", "actions.toml and Send to"),
    ("config", "config.toml"),
    ("index", "insights cache"),
    ("git", "all git reads, via gix"),
    ("types", "shared types, each defined once"),
    ("schwarzwald", "the theme and its palette"),
];

/// The crate dependency diagram, from the workspace's own edges. Only crates the binary
/// ships are drawn: a crate nothing depends on outside dev-dependencies (testkit) is test
/// support, not a layer. A shipped crate without a role in `ROLES` fails generation, so a
/// new crate can never show up as an empty box.
pub fn crate_diagram(ws: &Workspace) -> anyhow::Result<String> {
    let short = |n: &str| n.strip_prefix("gitterrace-").unwrap_or(n).to_owned();
    let mut s = String::from("```mermaid\nflowchart TD\n");
    let mut edges = ws.reduced_edges();
    // What the binaries reach through normal dependencies: the shipped layers.
    let mut shipped: Vec<&str> = ws
        .crates()
        .filter(|p| !p.bins.is_empty())
        .map(|p| p.name.as_str())
        .collect();
    let mut i = 0;
    while let Some(n) = shipped.get(i).copied() {
        for d in ws.packages.get(n).into_iter().flat_map(|p| &p.deps) {
            if !shipped.contains(&d.as_str()) {
                shipped.push(d);
            }
        }
        i += 1;
    }
    edges.retain(|(a, b)| shipped.contains(&a.as_str()) && shipped.contains(&b.as_str()));
    edges.sort();
    let mut names: Vec<String> = edges
        .iter()
        .flat_map(|(a, b)| [a.clone(), b.clone()])
        .collect();
    names.sort();
    names.dedup();
    for n in &names {
        let name = short(n);
        let Some((_, role)) = ROLES.iter().find(|(k, _)| *k == name) else {
            anyhow::bail!(
                "crate `{n}` has no role for the architecture diagram: add it to ROLES in \
                 xtask/src/readme_agents_md.rs"
            );
        };
        let _ = writeln!(s, "  {name}[\"{name}: {role}\"]");
    }
    for (from, to) in edges {
        let _ = writeln!(s, "  {} --> {}", short(&from), short(&to));
    }
    s.push_str("```\n");
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard class: every box in the architecture diagram names a role, and a crate only
    /// tests use (testkit, or any later one) is never drawn as a layer.
    #[test]
    fn the_diagram_draws_shipped_crates_each_with_a_role() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let ws = crate::facts::load(&root).expect("workspace");
        let d = crate_diagram(&ws).expect("diagram");
        assert!(!d.contains(": \"]"), "a box without a role:\n{d}");
        assert!(!d.contains("testkit"), "a test-only crate is drawn:\n{d}");
        assert!(d.contains("config[\"config: config.toml\"]"), "{d}");
    }

    #[test]
    fn installer_and_curl_line_derive_from_metadata() {
        assert_eq!(
            installer_url("https://github.com/o/r", "r"),
            "https://github.com/o/r/releases/latest/download/r-installer.sh"
        );
        assert_eq!(
            curl_install("https://example.dev/"),
            "curl -fsSL https://example.dev/install.sh | sh"
        );
    }
}
