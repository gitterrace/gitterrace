//! `docs/cli.md`, rendered from the clap definition the binary parses with: every command, its
//! usage and each argument with its value, help, default and values.

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

/// One list item per argument: the name with its value as code, then the help, then the
/// default and the accepted values only when there are any. A list reads at any width, where a
/// table with mostly empty columns squeezes the help.
fn table(s: &mut String, args: &[&Arg]) {
    for a in args {
        let mut line = name(a);
        let help = cell(
            &a.get_long_help()
                .or_else(|| a.get_help())
                .map_or_else(String::new, ToString::to_string),
        );
        if !help.is_empty() {
            let _ = write!(line, ": {}", help.trim_end_matches('.'));
            line.push('.');
        }
        let default: Vec<String> = a
            .get_default_values()
            .iter()
            .map(|v| v.to_string_lossy().into_owned())
            .filter(|v| a.get_action().takes_values() || v != "false")
            .collect();
        if !default.is_empty() {
            let _ = write!(
                line,
                " Default {}.",
                join(default.iter().map(|d| d.as_str().into()))
            );
        }
        let values = join(
            a.get_possible_values()
                .iter()
                .filter(|v| !v.is_hide_set())
                .map(|v| v.get_name().to_owned().into()),
        );
        if !values.is_empty() {
            let _ = write!(line, " One of {values}.");
        }
        let _ = writeln!(s, "{}", crate::out::wrap_item(&line));
    }
}

fn name(a: &Arg) -> String {
    let value = kind(a);
    match (a.get_long(), a.get_short()) {
        (Some(l), Some(sh)) => format!("`-{sh}`, `--{l}{value}`"),
        (Some(l), None) => format!("`--{l}{value}`"),
        _ => format!("`{}{}`", value_name(a), many(a)),
    }
}

/// The value a flag takes, as ` <N>`, empty for a switch.
fn kind(a: &Arg) -> String {
    if a.get_action().takes_values() {
        format!(" {}{}", value_name(a), many(a))
    } else {
        String::new()
    }
}

fn value_name(a: &Arg) -> String {
    let v = a
        .get_value_names()
        .and_then(|n| n.first().map(ToString::to_string))
        .unwrap_or_else(|| a.get_id().as_str().to_uppercase());
    format!("<{v}>")
}

fn many(a: &Arg) -> &'static str {
    if a.get_num_args().is_some_and(|r| r.max_values() > 1) {
        "..."
    } else {
        ""
    }
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

#[cfg(test)]
mod tests {
    use crate::out::Out;

    fn body(o: Out) -> String {
        match o {
            Out::File { body, .. } | Out::Block { body, .. } => body,
            Out::Bytes { .. } => String::new(),
        }
    }

    /// Guard class: a generated reference table with a column no row fills, which only takes
    /// width from the text a reader wants. Covers every generated reference page.
    #[test]
    fn no_generated_reference_table_has_an_always_empty_column() {
        let mut bad = Vec::new();
        for (page, md) in [
            ("cli", body(super::render())),
            ("configuration", body(crate::config_md::render())),
            ("api", body(crate::api_md::render())),
        ] {
            for c in empty_columns(&md) {
                bad.push(format!("{page}: {c}"));
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
        assert!(!empty_columns("| a | b |\n| --- | --- |\n| x |  |\n").is_empty());
    }

    /// Every column of every Markdown table in `md` that is empty in all of its rows, as
    /// "header (table at line N)". Empty when every column carries something.
    fn empty_columns(md: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let lines: Vec<&str> = md.lines().collect();
        let cells = |l: &str| -> Vec<String> {
            l.trim()
                .trim_matches('|')
                .split(" | ")
                .map(|c| c.trim().to_owned())
                .collect()
        };
        let mut i = 0;
        while i + 1 < lines.len() {
            let delim = lines[i + 1].trim();
            if lines[i].trim_start().starts_with('|')
                && delim.starts_with('|')
                && delim.contains("---")
            {
                let head = cells(lines[i]);
                let mut filled = vec![false; head.len()];
                let mut j = i + 2;
                while j < lines.len() && lines[j].trim_start().starts_with('|') {
                    for (k, c) in cells(lines[j]).iter().enumerate() {
                        if k < filled.len() && !c.is_empty() {
                            filled[k] = true;
                        }
                    }
                    j += 1;
                }
                for (k, f) in filled.iter().enumerate() {
                    if !f {
                        bad.push(format!("{} (table at line {})", head[k], i + 1));
                    }
                }
                i = j;
            } else {
                i += 1;
            }
        }
        bad
    }
}
