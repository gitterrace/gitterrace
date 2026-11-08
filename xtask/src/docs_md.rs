//! Generated blocks of `docs/actions.md`, `docs/architecture.md` and `docs/benchmarks.md`.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use gitterrace_actions::Config;
use gitterrace_actions::schema::{
    ACTION_KEYS, ENV_VARS, Item, KINDS, MODIFIERS, PAYLOAD_FIELDS, PLACEHOLDERS, SCRIPT_KEYS,
    SEND_KEYS, SEND_PLACEHOLDERS, kind_name,
};
use gitterrace_actions::send::DENIED_SCHEMES;

use crate::action_scripts::Script;
use crate::facts::Workspace;
use crate::out::Out;
use crate::readme_agents_md::crate_diagram;

const ACTIONS: &str = "docs/actions.md";
const ARCH: &str = "docs/architecture.md";
const BENCH: &str = "docs/benchmarks.md";

pub fn render(root: &Path, ws: &Workspace, scripts: &[Script]) -> Result<Vec<Out>> {
    Ok(vec![
        Out::block(ACTIONS, "kinds", kinds()),
        Out::block(ACTIONS, "payload", payload()),
        Out::block(ACTIONS, "env", env()),
        Out::block(ACTIONS, "action-keys", action_keys()),
        Out::block(ACTIONS, "placeholders", placeholders()),
        Out::block(ACTIONS, "starter", starter()),
        Out::block(ACTIONS, "send", send()),
        Out::block(ACTIONS, "scripts", script_table(scripts)),
        Out::block(ARCH, "crates", crate_diagram(ws)?),
        Out::block(ARCH, "routes", routes()),
        Out::block(ARCH, "languages", languages()),
        Out::block(ARCH, "cache", cache()),
        Out::block(BENCH, "corpus", bench_report(root, "--corpus")?),
        Out::block(BENCH, "results", bench_report(root, "--print")?),
    ])
}

/// A benchmark block from `scripts/bench-report.py`: `--corpus` lists the repos with their source
/// links from `tests/bench/corpus.toml`, `--print` renders the newest result file.
fn bench_report(root: &Path, flag: &str) -> Result<String> {
    let out = std::process::Command::new("python3")
        .args(["scripts/bench-report.py", flag])
        .current_dir(root)
        .output()
        .context("running scripts/bench-report.py")?;
    anyhow::ensure!(
        out.status.success(),
        "bench-report.py: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).context("bench-report.py output")
}

fn kinds() -> String {
    let rows: Vec<String> = KINDS
        .iter()
        .map(|(k, doc)| {
            format!(
                "`{}` ({})",
                kind_name(*k),
                doc.trim_end_matches('.').to_lowercase()
            )
        })
        .collect();
    format!("{}.\n", rows.join(", "))
}

fn payload() -> String {
    let mut s =
        String::from("| Field | Type | Always there | Description |\n| --- | --- | --- | --- |\n");
    for f in PAYLOAD_FIELDS {
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} |",
            f.name,
            f.ty,
            yes(f.required),
            f.doc
        );
    }
    s
}

fn env() -> String {
    let names: Vec<String> = ENV_VARS.iter().map(|(k, _)| format!("`{k}`")).collect();
    format!("{}.\n", names.join(", "))
}

const fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn key_table(title: &str, intro: &str, items: &[Item]) -> String {
    let mut s = format!(
        "### {title}\n\n{intro}\n\n| Key | Type | Required | Default | Description |\n| --- | --- \
        | --- | --- | --- |\n"
    );
    for i in items {
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} | {} |",
            i.name,
            i.ty,
            yes(i.required),
            i.default,
            i.doc
        );
    }
    s
}

fn placeholders() -> String {
    let mut s =
        String::from("| Placeholder | From the repository | Value |\n| --- | --- | --- |\n");
    for (name, repo, doc) in PLACEHOLDERS {
        let _ = writeln!(s, "| `{{{name}}}` | {} | {doc} |", yes(repo));
    }
    s.push_str("\n| Modifier | Effect |\n| --- | --- |\n");
    for (name, doc) in MODIFIERS {
        let _ = writeln!(s, "| `{{name:{name}}}` | {doc} |");
    }
    s
}

/// One block of the starter file, read as text so the generator needs no second schema.
pub struct Entry {
    pub table: &'static str,
    pub id: String,
    pub enabled: bool,
    pub about: String,
    pub vars: Vec<String>,
    /// The block has a `lookup` template.
    pub lookup: bool,
}

/// The blocks of `defaults/actions.toml`: a chunk is the comment lines right above a
/// `[[send]]` or `[[action]]` header, down to the next blank line.
pub fn starter_entries() -> Vec<Entry> {
    let value = |text: &str, key: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once('=')?;
            (k.trim() == key).then(|| v.trim().trim_matches('"').to_owned())
        })
    };
    Config::STARTER
        .split("\n\n")
        .filter_map(|chunk| {
            let table = if chunk.contains("\n[[send]]") || chunk.starts_with("[[send]]") {
                "send"
            } else if chunk.contains("\n[[action]]") || chunk.starts_with("[[action]]") {
                "action"
            } else {
                return None;
            };
            let about: Vec<&str> = chunk.lines().filter_map(|l| l.strip_prefix("# ")).collect();
            let body: Vec<&str> = chunk.lines().filter(|l| !l.starts_with('#')).collect();
            let text = body.join("\n");
            let vars = value(&text, "vars")
                .map(|v| {
                    v.trim_matches(['{', '}', ' '])
                        .split(',')
                        .filter_map(|kv| kv.split_once('=').map(|(k, _)| format!("`{}`", k.trim())))
                        .collect()
                })
                .unwrap_or_default();
            Some(Entry {
                table,
                id: value(&text, "id")?,
                enabled: value(&text, "enabled").is_none_or(|e| e == "true"),
                about: about.join(" "),
                vars,
                lookup: value(&text, "lookup").is_some(),
            })
        })
        .collect()
}

fn starter() -> String {
    let mut s = String::from(
        "| Entry | Kind | On | What it does | Needs in `vars` | Commit lookup |\n| --- | --- | --- \
        | --- | --- | --- |\n",
    );
    let entries = starter_entries();
    for e in &entries {
        let _ = writeln!(
            s,
            "| `{}` | {} | {} | {} | {} | {} |",
            e.id,
            if e.table == "send" {
                "Send to"
            } else {
                "action"
            },
            yes(e.enabled),
            e.about,
            e.vars.join(", "),
            yes(e.lookup)
        );
    }
    // Linked, not copied: `git terrace init` writes the real file, a copy doubled the page.
    s.push_str(
        "\nThe whole file, as `git terrace init` writes it: \
         [defaults/actions.toml](../crates/gitterrace-actions/defaults/actions.toml).\n",
    );
    s
}

fn send() -> String {
    let mut s = String::new();
    s.push_str("\n| Placeholder | Value |\n| --- | --- |\n");
    for (name, doc) in SEND_PLACEHOLDERS {
        let _ = writeln!(s, "| `{{{name}}}` | {doc} |");
    }
    let denied: Vec<String> = DENIED_SCHEMES.iter().map(|d| format!("`{d}:`")).collect();
    let _ = writeln!(s, "\nRefused schemes: {}.\n", denied.join(", "));
    s.push_str(&key_table(
        "`[[send]]`",
        "One Send to target: a `url` template.",
        &SEND_KEYS,
    ));
    s
}

/// What an `[[action]]` is, for the docs and the website.
pub const ACTION_ABOUT: &str = "One menu entry. It runs either its own `command` or a `script`.";

fn action_keys() -> String {
    [
        key_table(
            "`[[script]]`",
            "A command line written once, for several actions.",
            &SCRIPT_KEYS,
        ),
        key_table("`[[action]]`", ACTION_ABOUT, &ACTION_KEYS),
    ]
    .join("\n")
}

fn script_table(scripts: &[Script]) -> String {
    let mut s = String::from("| Script | Does | Needs |\n| --- | --- | --- |\n");
    for sc in scripts {
        let _ = writeln!(
            s,
            "| [`{f}`](actions/{f}) | {} | {} |",
            sc.summary,
            sc.needs,
            f = sc.file
        );
    }
    s
}

fn routes() -> String {
    let mut s = String::from("| Route | Shows |\n| --- | --- |\n");
    for p in gitterrace_web::sitemap::PAGES {
        let _ = writeln!(s, "| {} | {} |", p.shown, p.does);
    }
    s
}

/// The highlighted languages as one comma-separated paragraph, from the syntax set the binary
/// ships, so the list cannot drift from what the code view does.
fn languages() -> String {
    let names = gitterrace_web::highlighted_languages();
    format!("{} languages: {}.\n", names.len(), names.join(", "))
}

fn cache() -> String {
    let mut s = String::from("| File | Holds |\n| --- | --- |\n");
    for (name, doc) in gitterrace_index::CACHE_FILES {
        let _ = writeln!(s, "| `{name}` | {doc} |");
    }
    let _ = write!(
        s,
        "\nThe format version is `{}`.\n",
        gitterrace_index::FORMAT_VERSION
    );
    s
}
