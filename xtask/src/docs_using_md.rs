//! Generated blocks of `docs/using.md` and `docs/mcp.md`.

use std::fmt::Write as _;

/// The commit filter's keywords, from the table the parser and the field's completions read.
pub(crate) fn log_filter() -> String {
    let mut s = String::from("| Syntax | Matches |\n| --- | --- |\n");
    for k in gitterrace_index::query::KEYWORDS {
        let _ = writeln!(s, "| `{}` | {} |", k.syntax.replace('|', "\\|"), k.description);
    }
    s.push_str("\nA period is `1w`, `1m`, `3m` or `1y`, counted back from the newest commit.\n");
    s
}

/// The terms of a review plan's risk with their points, from the weights the plan scores with.
pub(crate) fn risk_terms() -> String {
    let mut s = String::from("| Term | Points |\n| --- | --- |\n");
    for (name, rule) in gitterrace_api::risk_terms() {
        let _ = writeln!(s, "| `{name}` | {rule} |");
    }
    s
}

/// The `review_change` prompt's arguments, from the live prompt registry.
pub(crate) fn prompt_args() -> String {
    let docs = gitterrace_api::McpServer::prompt_docs();
    let mut s = String::from("| Argument | Meaning |\n| --- | --- |\n");
    for p in docs.iter().filter(|p| p.name == "review_change") {
        for a in &p.params {
            let _ = writeln!(s, "| `{}` | {} |", a.name, a.description);
        }
    }
    s
}

/// The Keys table, from the list the `?` sheet in `nav.js` renders. Only the keys with a guide
/// wording appear: the sheet has the rest.
pub(crate) fn keys() -> String {
    let mut s = String::from("| Key | Does |\n| --- | --- |\n");
    for k in gitterrace_web::shortcuts() {
        let Some(doc) = &k.doc else { continue };
        let _ = writeln!(s, "| {} | {doc} |", key_cell(&k.keys));
    }
    s
}

/// `g c` as "`g` then `c`", `j / k` as "`j`, `k`", one key as itself.
fn key_cell(keys: &str) -> String {
    if keys.contains(" / ") {
        return keys.split(" / ").map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ");
    }
    keys.split(' ').map(|k| format!("`{k}`")).collect::<Vec<_>>().join(" then ")
}

/// What the starter turns on by itself, as one sentence for the front page.
pub(crate) fn send_defaults() -> String {
    let on: Vec<String> = gitterrace_actions::examples::enabled_by_default()
        .iter()
        .map(|(_, label)| format!("**{label}**"))
        .collect();
    format!("Out of the box only {} works.\n", on.join(" and "))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gitterrace_api::McpServer;

    /// The source of the shared API types, where a response field is declared.
    fn types_source(root: &std::path::Path) -> String {
        let mut all = String::new();
        let mut dirs = vec![root.join("crates/gitterrace-types/src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).expect("types dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if let Ok(text) = std::fs::read_to_string(&path) {
                    all.push_str(&text);
                }
            }
        }
        all
    }

    /// Every `snake_case` word in backticks on the agent page is a tool, a prompt, one of their
    /// arguments or a response field of the shared types, so a renamed tool fails here instead of
    /// misleading an agent.
    #[test]
    fn every_name_in_the_agent_page_is_in_the_mcp_registry() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let page = std::fs::read_to_string(root.join("docs/mcp.md")).expect("docs/mcp.md");
        let mut known: BTreeSet<String> = BTreeSet::new();
        for d in McpServer::tool_docs().into_iter().chain(McpServer::prompt_docs()) {
            known.insert(d.name.clone());
            known.extend(d.params.iter().map(|p| p.name.clone().into_owned()));
        }
        let fields = types_source(&root);
        known.extend(gitterrace_api::insight_list().map(|(name, _)| (*name).to_owned()));
        let mut bad = Vec::new();
        for word in page.split('`').skip(1).step_by(2) {
            let snake =
                word.contains('_') && word.chars().all(|c| c.is_ascii_lowercase() || c == '_');
            if snake && !known.contains(word) && !fields.contains(&format!("pub {word}:")) {
                bad.push(word.to_owned());
            }
        }
        bad.sort();
        bad.dedup();
        assert!(bad.is_empty(), "docs/mcp.md names things the MCP registry lacks: {bad:?}");
    }
}
