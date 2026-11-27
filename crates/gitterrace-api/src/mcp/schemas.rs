//! Schemas of the arguments that the registries of the insights and the filter describe.

use crate::insights::INSIGHTS;

/// The `name` argument: an enum of the registry's names, each with its summary, so the model
/// is told what exists and a new insight shows up here without another list.
pub(super) fn name_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    let list: Vec<String> =
        INSIGHTS.iter().map(|i| format!("`{}`: {}", i.name, i.summary)).collect();
    schemars::json_schema!({
        "type": "string",
        "enum": INSIGHTS.iter().map(|i| i.name).collect::<Vec<_>>(),
        "description": format!("The insight to compute. {}", list.join(" ")),
    })
}

/// The `query` argument: the commit filter of the web page, its grammar read from the keyword
/// registry so the model is told what the parser accepts and the two cannot drift.
pub(super) fn query_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["string", "null"],
        "description": format!(
            "Commit filter, every term must match, e.g. `author:ann after:3m !merge`. Same \
             text as the web filter. Together with `period` and `path`, which read as \
             `after:` and `path:` terms. {}",
            gitterrace_index::query::grammar_line()
        ),
    })
}
