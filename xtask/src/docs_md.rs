//! Generated blocks of `docs/actions.md`, `docs/architecture.md`, `docs/benchmarks.md` and
//! `docs/using.md`.

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
const USING: &str = "docs/using.md";

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
        Out::block(ACTIONS, "limits", limits()),
        Out::block(ARCH, "crates", crate_diagram(ws)?),
        Out::block(ARCH, "routes", routes()),
        Out::block(ARCH, "languages", languages()),
        Out::block(ARCH, "cache", cache()),
        Out::block(BENCH, "corpus", bench_report(root, "--corpus")?),
        Out::block(BENCH, "results", bench_report(root, "--print")?),
        Out::block(USING, "log-filter", log_filter()),
    ])
}

/// A benchmark block from `scripts/bench/bench-report.py`: `--corpus` lists the repos with their
/// source links from `tests/bench/corpus.toml`, `--print` renders the newest result file.
fn bench_report(root: &Path, flag: &str) -> Result<String> {
    let out = std::process::Command::new("python3")
        .args(["scripts/bench/bench-report.py", flag])
        .current_dir(root)
        .output()
        .context("running scripts/bench/bench-report.py")?;
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
            format!("`{}` ({})", kind_name(*k), doc.trim_end_matches('.').to_lowercase())
        })
        .collect();
    format!("{}.\n", rows.join(", "))
}

fn payload() -> String {
    let mut s =
        String::from("| Field | Type | Always there | Description |\n| --- | --- | --- | --- |\n");
    for f in PAYLOAD_FIELDS {
        let _ = writeln!(s, "| `{}` | {} | {} | {} |", f.name, f.ty, yes(f.required), f.doc);
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
            })
        })
        .collect()
}

/// The widest a generated markdown table row may be, so the page stays readable and short.
pub const TABLE_ROW_MAX: usize = 100;

/// The first sentence of `about`, cut at a word to fit `budget` columns.
fn summary(about: &str, budget: usize) -> String {
    let first = about.split_once(". ").map_or(about, |(f, _)| f).trim_end_matches('.');
    if first.chars().count() <= budget {
        return first.to_owned();
    }
    let mut out = String::new();
    for w in first.split(' ') {
        if out.chars().count() + w.chars().count() + 4 > budget {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out.push_str("...");
    out
}

fn starter() -> String {
    let mut s = String::from(
        "| Entry | Kind | On | What it does | `vars` |\n| --- | --- | --- | --- | --- |\n",
    );
    let entries = starter_entries();
    for e in &entries {
        let kind = if e.table == "send" { "send" } else { "action" };
        let head = format!("| `{}` | {kind} | {} | ", e.id, yes(e.enabled));
        let tail = format!(" | {} |", e.vars.join(", "));
        let budget = TABLE_ROW_MAX.saturating_sub(head.len() + tail.len());
        let _ = writeln!(s, "{head}{}{tail}", summary(&e.about, budget));
    }
    // Linked, not copied: `git terrace init` writes the real file, a copy doubled the page.
    s.push_str(
        "\nA cell ending in `...` is cut short. The whole text of every entry is in the file \
         `git terrace init` writes: \
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
    s.push_str(&key_table("`[[send]]`", "One Send to target: a `url` template.", &SEND_KEYS));
    s
}

/// The whole Security list of the actions page: the file and field rules, then the fixed run
/// limits from the runner and loader constants. One generated list, so the page never shows
/// a hand list and a generated one as two lists.
fn limits() -> String {
    use gitterrace_actions::config::MAX_TIMEOUT;
    use gitterrace_actions::runner::{MAX_RUNNING, PER_MINUTE};
    let output = gitterrace_config::keys()
        .iter()
        .find(|k| k.id == "limits.action_output_kb")
        .map_or_else(String::new, |k| k.default.clone());
    let items = [
        "The file must be a regular file outside every served repository. On Unix it and its \
         folder must also be owned by you and not writable by group or others. Windows checks \
         neither and relies on the profile folder's ACLs (see [Windows](#windows)). Actions are \
         off on a public server, on a non-loopback bind and with `--no-actions`, and never \
         offered to MCP."
            .to_owned(),
        "Every field is untrusted repository text. In a script, quote it and never `eval` it."
            .to_owned(),
        format!(
            "No shell between gitterrace and the program: a placeholder is one argument, never \
             parsed. Scrubbed environment, timeout, at most {MAX_RUNNING} runs at once, \
             {PER_MINUTE} a minute."
        ),
        format!(
            "A `timeout` is at most {} s. That ceiling is fixed and no setting raises it. Each \
             stream of output keeps its first `limits.action_output_kb` (default {output} KiB) \
             and drops the rest.",
            MAX_TIMEOUT.as_secs()
        ),
    ];
    items.iter().fold(String::new(), |mut s, i| {
        let _ = writeln!(s, "{}", crate::out::wrap_item(i));
        s
    })
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
        let _ =
            writeln!(s, "| [`{f}`](actions/{f}) | {} | {} |", sc.summary, sc.needs, f = sc.file);
    }
    s
}

/// The commit filter's keywords, from the table the parser and the field's completions read.
fn log_filter() -> String {
    let mut s = String::from("| Syntax | Matches |\n| --- | --- |\n");
    for k in gitterrace_index::query::KEYWORDS {
        let _ = writeln!(s, "| `{}` | {} |", k.syntax.replace('|', "\\|"), k.description);
    }
    s.push_str("\nA period is `1w`, `1m`, `3m` or `1y`, counted back from the newest commit.\n");
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
        "\nThe format version is `{}`, stored as `v{}`.\n",
        gitterrace_index::FORMAT_VERSION,
        gitterrace_index::FORMAT_VERSION
    );
    s
}

#[cfg(test)]
mod tests {
    use super::{TABLE_ROW_MAX, render};

    /// Blocks whose rows are still wider than `TABLE_ROW_MAX`, with their widest row today. A
    /// ratchet that only falls: a block may not get wider, and when it gets narrower the cap must
    /// be lowered to match (or the entry dropped once it fits). Any block not listed must fit.
    const STILL_WIDE: &[(&str, usize)] = &[
        ("send", 283),
        ("placeholders", 266),
        ("action-keys", 263),
        ("payload", 154),
        ("scripts", 322),
        ("cache", 165),
        ("routes", 270),
    ];
    use crate::facts;
    use crate::out::Out;

    /// No generated markdown table row is wider than `TABLE_ROW_MAX`, in any block. A long cell
    /// grows the page past its line cap once the rows wrap.
    #[test]
    fn generated_table_rows_stay_narrow() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let ws = facts::load(&root).expect("workspace facts");
        let scripts = crate::action_scripts::load(&root).expect("action scripts");
        let mut bad = Vec::new();
        for out in render(&root, &ws, &scripts).expect("render") {
            let Out::Block { path, id, body } = out else {
                continue;
            };
            let widest = body
                .lines()
                .filter(|l| l.starts_with('|'))
                .map(|l| l.chars().count())
                .max()
                .unwrap_or(0);
            let cap = STILL_WIDE
                .iter()
                .find(|(name, _)| *name == id)
                .map_or(TABLE_ROW_MAX, |(_, cap)| *cap);
            if widest > cap {
                let row = body.lines().find(|l| l.chars().count() == widest).unwrap_or("");
                bad.push(format!(
                    "{path} [{id}]: widest row is {widest} columns, cap {cap}: {}",
                    row.chars().take(40).collect::<String>()
                ));
            } else if cap > TABLE_ROW_MAX && widest < cap {
                bad.push(format!(
                    "{path} [{id}]: widest row shrank to {widest}, lower its STILL_WIDE cap from \
                     {cap} (drop the entry if it fits {TABLE_ROW_MAX})"
                ));
            }
        }
        assert!(bad.is_empty(), "generated table rows too wide:\n{}", bad.join("\n"));
    }
}
