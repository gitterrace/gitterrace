//! `docs/cli.md`, rendered from the clap definition the binary parses with: every command, its
//! usage and each argument with type, default, values and help.

use std::fmt::Write as _;

use clap::{Arg, Command, CommandFactory as _};

use crate::out::Out;
use crate::readme_agents_md::cli;

const PATH: &str = "docs/cli.md";

const INTRO: &str = "\
Everything the `git terrace` command line accepts, read from the same definition the binary
parses with. `git terrace --help` and `git terrace <command> --help` print the same text.
The global flags work on every command.
";

pub fn render() -> Out {
    let mut root = cli::Cli::command().bin_name("git terrace");
    root.build();
    let mut s = String::from(INTRO);
    s.push_str("\n## `git terrace`\n\n");
    command_body(&mut s, &root, true);
    let globals: Vec<&Arg> = visible(&root).filter(|a| a.is_global_set()).collect();
    s.push_str("\n## Global flags\n\n");
    table(&mut s, &globals);
    for sub in root.get_subcommands().filter(|c| c.get_name() != "help") {
        let _ = write!(s, "\n## `git terrace {}`\n\n", sub.get_name());
        command_body(&mut s, sub, false);
    }
    Out::block(PATH, "cli", s)
}

fn visible(c: &Command) -> impl Iterator<Item = &Arg> {
    c.get_arguments()
        .filter(|a| !a.is_hide_set() && a.get_id() != "help" && a.get_id() != "version")
}

fn command_body(s: &mut String, c: &Command, is_root: bool) {
    if let Some(about) = c.get_long_about().or_else(|| c.get_about()) {
        let _ = writeln!(s, "{about}\n");
    }
    let usage = c.clone().render_usage().to_string();
    let _ = writeln!(
        s,
        "```text\n{}\n```",
        usage.trim_start_matches("Usage: ").trim()
    );
    let own: Vec<&Arg> = visible(c)
        .filter(|a| is_root || !a.is_global_set())
        .collect();
    let own: Vec<&Arg> = if is_root {
        own.into_iter().filter(|a| !a.is_global_set()).collect()
    } else {
        own
    };
    if !own.is_empty() {
        s.push('\n');
        table(s, &own);
    }
}

fn table(s: &mut String, args: &[&Arg]) {
    s.push_str("| Argument | Type | Default | Values | Help |\n| --- | --- | --- | --- | --- |\n");
    for a in args {
        let _ = writeln!(
            s,
            "| {} | {} | {} | {} | {} |",
            name(a),
            kind(a),
            join(a.get_default_values().iter().map(|v| v.to_string_lossy())),
            join(
                a.get_possible_values()
                    .iter()
                    .filter(|v| !v.is_hide_set())
                    .map(|v| v.get_name().to_owned().into())
            ),
            cell(
                &a.get_long_help()
                    .or_else(|| a.get_help())
                    .map_or_else(String::new, ToString::to_string)
            ),
        );
    }
}

fn name(a: &Arg) -> String {
    match (a.get_long(), a.get_short()) {
        (Some(l), Some(sh)) => format!("`-{sh}`, `--{l}`"),
        (Some(l), None) => format!("`--{l}`"),
        _ => format!("`{}`", a.get_id().as_str().replace('_', "-")),
    }
}

fn kind(a: &Arg) -> String {
    if !a.get_action().takes_values() {
        return "flag".to_owned();
    }
    let v = a
        .get_value_names()
        .and_then(|n| n.first().map(ToString::to_string))
        .unwrap_or_else(|| a.get_id().as_str().to_uppercase());
    let many = a.get_num_args().is_some_and(|r| r.max_values() > 1);
    format!("`{v}`{}", if many { " (one or more)" } else { "" })
}

fn join<'a>(items: impl Iterator<Item = std::borrow::Cow<'a, str>>) -> String {
    let v: Vec<String> = items.map(|i| format!("`{i}`")).collect();
    v.join(", ")
}

fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}
