//! The TOML examples the docs print, cut out of the starter file and `defaults/examples.toml`.
//!
//! A doc never types an example. It names an entry here and `cargo xtask generate` copies the
//! entry's text, so the page cannot show a key the parser refuses or a value the starter no
//! longer holds. The tests below hold every entry to the key tables and every doc block to its
//! entry.

use crate::config::Config;

/// The examples that are not starter entries.
pub const EXAMPLES_FILE: &str = include_str!("../defaults/examples.toml");

/// Why an example block cannot be built.
#[derive(Debug, thiserror::Error)]
pub enum ExampleError {
    /// An id names no entry of the starter or the examples file.
    #[error("example {block} names `{id}`, which no entry has")]
    UnknownId {
        /// The block that names it.
        block: &'static str,
        /// The id nothing holds.
        id: &'static str,
    },
}

/// One generated example block: where it goes, and the entries it shows, in order.
pub struct Spec {
    /// The doc, relative to the repository root.
    pub doc: &'static str,
    /// The id of the `GENERATED` block.
    pub block: &'static str,
    /// Entry ids, looked up in the starter file first and then in [`EXAMPLES_FILE`].
    pub ids: &'static [&'static str],
    /// Print the `version = 1` line first, as a whole file starts.
    pub version: bool,
}

/// Every example block of the docs.
pub const SPECS: [Spec; 6] = [
    Spec { doc: "docs/actions.md", block: "example-send", ids: &["github-issue"], version: true },
    Spec { doc: "docs/actions.md", block: "example-webhook", ids: &["webhook"], version: false },
    Spec { doc: "docs/actions.md", block: "example-search", ids: &["search"], version: false },
    Spec { doc: "docs/actions.md", block: "example-agent-other", ids: &["aider"], version: false },
    Spec {
        doc: "docs/actions.md",
        block: "example-script",
        ids: &["obsidian", "obsidian-daily"],
        version: false,
    },
    Spec { doc: "docs/landing.md", block: "example-claude", ids: &["ask"], version: false },
];

/// One `[[send]]`, `[[script]]` or `[[action]]` block of a file.
struct Chunk {
    table: &'static str,
    id: String,
    /// The block as shown: no comments and no `enabled` line, which only the starter needs.
    shown: String,
    /// Every top-level key the block sets.
    keys: Vec<String>,
}

/// The blocks of `text`: a chunk is the lines between blank lines that holds a table header.
fn chunks(text: &str) -> Vec<Chunk> {
    text.split("\n\n")
        .filter_map(|chunk| {
            let body: Vec<&str> = chunk.lines().filter(|l| !l.starts_with('#')).collect();
            let table = ["send", "script", "action"]
                .into_iter()
                .find(|t| body.iter().any(|l| *l == format!("[[{t}]]")))?;
            let value: toml::Table = toml::from_str(&body.join("\n")).ok()?;
            let entry = value.get(table)?.as_array()?.first()?.as_table()?;
            let shown: Vec<&str> =
                body.iter().copied().filter(|l| !l.starts_with("enabled")).collect();
            Some(Chunk {
                table,
                id: entry.get("id")?.as_str()?.to_owned(),
                shown: shown.join("\n"),
                keys: entry.keys().cloned().collect(),
            })
        })
        .collect()
}

fn all_chunks() -> Vec<Chunk> {
    let mut all = chunks(Config::STARTER);
    all.extend(chunks(EXAMPLES_FILE));
    all
}

/// The text of one example block: the entries of `spec`, fenced as TOML.
///
/// # Errors
///
/// When an id of `spec` names no entry, so a doc cannot name an entry that is gone.
pub fn render(spec: &Spec) -> Result<String, ExampleError> {
    let all = all_chunks();
    let mut parts: Vec<String> = Vec::new();
    if spec.version {
        parts.push("version = 1".to_owned());
    }
    for id in spec.ids {
        let Some(c) = all.iter().find(|c| c.id == *id) else {
            return Err(ExampleError::UnknownId { block: spec.block, id });
        };
        parts.push(c.shown.clone());
    }
    Ok(format!("```toml\n{}\n```\n", parts.join("\n\n")))
}

/// Every entry the docs can show, as `(table, id, keys it sets)`, for a guard that holds the keys
/// to the key tables.
#[must_use]
pub fn entry_keys() -> Vec<(&'static str, String, Vec<String>)> {
    all_chunks().into_iter().map(|c| (c.table, c.id, c.keys)).collect()
}

/// The starter's send targets and actions that are on out of the box, as `(id, label)`.
#[must_use]
pub fn enabled_by_default() -> Vec<(String, String)> {
    let Ok(value) = toml::from_str::<toml::Table>(Config::STARTER) else {
        return Vec::new();
    };
    ["send", "action"]
        .iter()
        .filter_map(|t| value.get(*t).and_then(toml::Value::as_array))
        .flatten()
        .filter_map(toml::Value::as_table)
        .filter(|a| a.get("enabled").and_then(toml::Value::as_bool) != Some(false))
        .filter_map(|a| {
            Some((a.get("id")?.as_str()?.to_owned(), a.get("label")?.as_str()?.to_owned()))
        })
        .collect()
}

/// The starter's coding agents: the entries that run `agent.sh`, as `(id, label)`.
#[must_use]
pub fn agents() -> Vec<(String, String)> {
    let Ok(value) = toml::from_str::<toml::Table>(Config::STARTER) else {
        return Vec::new();
    };
    let Some(actions) = value.get("action").and_then(toml::Value::as_array) else {
        return Vec::new();
    };
    actions
        .iter()
        .filter_map(toml::Value::as_table)
        .filter(|a| {
            let first = a.get("command").and_then(toml::Value::as_array).and_then(|c| c.first());
            first.and_then(toml::Value::as_str).is_some_and(|p| p.ends_with("/agent.sh"))
        })
        .filter_map(|a| {
            Some((a.get("id")?.as_str()?.to_owned(), a.get("label")?.as_str()?.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{SPECS, Spec, agents, enabled_by_default, entry_keys, render};
    use crate::schema::{ACTION_KEYS, Item, SCRIPT_KEYS, SEND_KEYS};

    const DOCS: [(&str, &str); 2] = [
        ("docs/actions.md", include_str!("../../../docs/actions.md")),
        ("docs/landing.md", include_str!("../../../docs/landing.md")),
    ];

    fn names(items: &[Item]) -> BTreeSet<&'static str> {
        items.iter().map(|i| i.name).collect()
    }

    /// The body of the `GENERATED` block `id` of `doc`.
    fn block<'a>(doc: &'a str, id: &str) -> Option<&'a str> {
        let rest = doc.split(&format!("<!-- BEGIN GENERATED: {id} -->\n")).nth(1)?;
        rest.split("<!-- END GENERATED").next()
    }

    /// Every entry sets only keys its table documents, so a key the parser renamed or dropped
    /// fails here even before the example is loaded.
    #[test]
    fn every_example_key_is_a_documented_key_of_its_table() {
        let all = entry_keys();
        assert!(all.len() > 10, "found {} entries", all.len());
        for (table, id, keys) in &all {
            let known = match *table {
                "send" => names(&SEND_KEYS),
                "script" => names(&SCRIPT_KEYS),
                _ => names(&ACTION_KEYS),
            };
            for k in keys {
                assert!(
                    k == "id" || known.contains(k.as_str()),
                    "{table} {id}: key `{k}` is not in the {table} key table"
                );
            }
        }
    }

    #[test]
    fn entry_ids_are_unique_across_the_starter_and_the_examples() {
        let all = entry_keys();
        let ids: BTreeSet<&str> = all.iter().map(|c| c.1.as_str()).collect();
        assert_eq!(ids.len(), all.len(), "an id names two entries");
    }

    /// A doc shows each example as its block says, and holds no TOML fence of its own.
    #[test]
    fn every_doc_example_is_the_entry_it_names() {
        for spec in &SPECS {
            let doc = DOCS.iter().find(|d| d.0 == spec.doc).expect("doc").1;
            let body = block(doc, spec.block)
                .unwrap_or_else(|| panic!("{} has no block {}", spec.doc, spec.block));
            assert_eq!(
                body.trim(),
                render(spec).expect("known ids").trim(),
                "{} [{}] drifted from its entry: run make docs-gen",
                spec.doc,
                spec.block
            );
        }
        for (path, doc) in DOCS {
            let fences = doc.matches("```toml\n").count();
            let owned = SPECS.iter().filter(|s| s.doc == path).count();
            assert_eq!(fences, owned, "{path}: a toml example is typed by hand");
        }
    }

    #[test]
    fn example_with_an_unknown_id_fails() {
        let spec = Spec { doc: "d", block: "b", ids: &["no-such-entry"], version: false };
        assert!(render(&spec).is_err());
        assert!(render(&SPECS[1]).expect("known id").contains("curl"));
    }

    /// A fence outside a generated block holds no `[[send]]`, `[[action]]` or `[[script]]`
    /// header under any language tag, or it drifts like a typed example.
    #[test]
    fn actions_doc_has_no_hand_written_entry_fence() {
        let (mut in_block, mut in_fence, mut bad) = (false, false, Vec::new());
        for (n, line) in DOCS[0].1.lines().enumerate() {
            if line.starts_with("<!-- BEGIN GENERATED") {
                in_block = true;
            } else if line.starts_with("<!-- END GENERATED") {
                in_block = false;
            } else if line.starts_with("```") {
                in_fence = !in_fence;
            } else if in_fence
                && !in_block
                && ["[[send]]", "[[action]]", "[[script]]"].contains(&line.trim())
            {
                bad.push(format!("line {}: {} in a hand-written fence", n + 1, line.trim()));
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn the_agents_are_the_starter_entries_that_run_agent_sh() {
        let ids: Vec<String> = agents().into_iter().map(|a| a.0).collect();
        assert_eq!(ids, ["claude", "codex", "kiro", "gemini"]);
    }

    #[test]
    fn only_mail_is_on_in_the_starter() {
        let ids: Vec<String> = enabled_by_default().into_iter().map(|a| a.0).collect();
        assert_eq!(ids, ["mail"]);
    }
}
