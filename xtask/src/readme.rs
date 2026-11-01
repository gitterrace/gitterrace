//! The generated blocks of `README.md` and `AGENTS.md`, and the crate and cache facts
//! `docs/architecture.md` shows. Everything here is read from the workspace or from the
//! registries the code itself uses.

use std::fmt::Write as _;

use clap::CommandFactory as _;
use gitterrace_actions::schema::{KINDS, kind_name};

use crate::facts::Workspace;
use crate::out::Out;
use crate::scripts::Script;

#[path = "../../crates/gitterrace/src/cli.rs"]
#[allow(
    dead_code,
    reason = "xtask only reads the definition, never the parsed values"
)]
mod cli;

const README: &str = "README.md";

pub fn render(ws: &Workspace, scripts: &[Script]) -> Vec<Out> {
    let bin = ws.packages.get("gitterrace");
    let mut install = String::new();
    if let Some(p) = bin {
        let names: Vec<String> = p.bins.iter().map(|b| format!("`{b}`")).collect();
        let _ = writeln!(
            install,
            "Every route installs {} names for one program: {}. git runs `git-terrace` as \
             `git terrace`.",
            p.bins.len(),
            names.join(" and ")
        );
    }
    let msrv = bin.map_or_else(String::new, |p| {
        format!("Rust {} or newer.", p.rust_version)
    });

    let kinds: Vec<String> = KINDS
        .iter()
        .map(|(k, _)| format!("`{}`", kind_name(*k)))
        .collect();
    let mut actions = format!(
        "Built in, no configuration: **Copy permalink** and **Copy as markdown**. Your own \
         actions run a program of yours on a selection, offered on {}. Examples in \
         [`docs/actions/`](docs/actions/):\n\n",
        kinds.join(", ")
    );
    for s in scripts {
        let _ = writeln!(
            actions,
            "- [`{}`](docs/actions/{}): {}",
            s.file, s.file, s.summary
        );
    }

    vec![
        Out::block(README, "msrv", msrv),
        Out::block(README, "install", install),
        Out::block(README, "commands", commands()),
        Out::block(README, "actions", actions),
        Out::block("AGENTS.md", "crates", crate_table(ws)),
    ]
}

/// One line per subcommand and flag, from the clap definition.
fn commands() -> String {
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
    let width = rows.iter().map(|(a, _)| a.len()).max().unwrap_or(0);
    let mut s = String::from("```\n");
    for (a, b) in rows {
        let _ = writeln!(s, "{a:width$}  {b}");
    }
    s.push_str("```\n");
    s
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
        "gitterrace-git",
        "gitterrace-index",
        "gitterrace-config",
        "gitterrace-actions",
        "gitterrace-api",
        "gitterrace-cards",
        "gitterrace-web",
        "gitterrace",
    ];
    let mut names: Vec<&str> = ws.crates().map(|p| p.name.as_str()).collect();
    names.sort_by_key(|n| order.iter().position(|o| o == n).unwrap_or(order.len()));
    for n in names {
        let p = &ws.packages[n];
        let extra = if p.bins.is_empty() {
            String::new()
        } else {
            format!(" Binaries `{}`.", p.bins.join("` and `"))
        };
        let _ = writeln!(s, "| `{}` | {}{extra} |", p.name, p.description);
    }
    s
}

/// The crate dependency diagram, from the workspace's own edges.
pub fn crate_diagram(ws: &Workspace) -> String {
    let id = |n: &str| n.replace('-', "_");
    let mut s = String::from("```mermaid\nflowchart TD\n");
    for p in ws.crates() {
        let short = p.name.strip_prefix("gitterrace-").unwrap_or(&p.name);
        let _ = writeln!(
            s,
            "  {}[\"{}\"]",
            id(&p.name),
            if p.name == "gitterrace" {
                "gitterrace (CLI)"
            } else {
                short
            }
        );
    }
    let mut edges = ws.reduced_edges();
    edges.sort();
    for (from, to) in edges {
        let _ = writeln!(s, "  {} --> {}", id(&from), id(&to));
    }
    s.push_str("```\n");
    s
}
