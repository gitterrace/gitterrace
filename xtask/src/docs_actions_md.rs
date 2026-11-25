//! Generated blocks of `docs/actions.md`.

use std::fmt::Write as _;

use anyhow::{Context, Result};

use gitterrace_actions::schema::{
    ACTION_KEYS, ENV_VARS, Item, KINDS, MODIFIERS, PAYLOAD_FIELDS, PLACEHOLDERS, SCRIPT_KEYS,
    SEND_KEYS, SEND_PLACEHOLDERS,
};
use gitterrace_actions::send::DENIED_SCHEMES;
use gitterrace_actions::{Config, kind_name};

use crate::action_scripts::Script;

pub(crate) fn kinds() -> String {
    let rows: Vec<String> = KINDS
        .iter()
        .map(|(k, doc)| {
            format!("`{}` ({})", kind_name(*k), doc.trim_end_matches('.').to_lowercase())
        })
        .collect();
    format!("{}.\n", rows.join(", "))
}

pub(crate) fn payload() -> String {
    let mut s =
        String::from("| Field | Type | Always there | Description |\n| --- | --- | --- | --- |\n");
    for f in PAYLOAD_FIELDS {
        let _ = writeln!(s, "| `{}` | {} | {} | {} |", f.name, f.ty, yes(f.required), f.doc);
    }
    s
}

pub(crate) fn env() -> String {
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

pub(crate) fn placeholders() -> String {
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

pub(crate) fn starter() -> String {
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

pub(crate) fn send() -> String {
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
pub(crate) fn limits() -> String {
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

pub(crate) fn action_keys() -> String {
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

pub(crate) fn script_table(scripts: &[Script]) -> String {
    let mut s = String::from("| Script | Does | Needs |\n| --- | --- | --- |\n");
    for sc in scripts {
        let _ =
            writeln!(s, "| [`{f}`](actions/{f}) | {} | {} |", sc.summary, sc.needs, f = sc.file);
    }
    s
}

/// A toml fence holding the starter chunks with these ids, verbatim and without their comment
/// lines. An id the starter lacks fails generation, so a doc cannot name an entry that is gone.
pub(crate) fn example(ids: &[&str]) -> Result<String> {
    let mut parts = Vec::new();
    for id in ids {
        let want = format!("id=\"{id}\"");
        let chunk = Config::STARTER
            .split("\n\n")
            .find(|c| {
                let mut body = c.lines().filter(|l| !l.starts_with('#'));
                body.next().is_some_and(|h| h.starts_with("[["))
                    && body.any(|l| l.split_whitespace().collect::<String>() == want)
            })
            .with_context(|| format!("docs example: no starter entry with id `{id}`"))?;
        let body: Vec<&str> = chunk.lines().filter(|l| !l.starts_with('#')).collect();
        parts.push(body.join("\n"));
    }
    Ok(format!("```toml\n{}\n```\n", parts.join("\n\n")))
}
