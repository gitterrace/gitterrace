//! The keyword table: what the parser accepts, what the docs list and what the filter field
//! offers as completion, from one list.

#[derive(Debug, Clone, Copy)]
pub struct KeywordSpec {
    /// The literal before the `:`, or the whole bare word for a flag such as `merge`.
    pub keyword: &'static str,
    /// The full form, `author:<name or email>`.
    pub syntax: &'static str,
    pub description: &'static str,
    /// A valid expression the parser must accept, `None` for the operator rows.
    pub example: Option<&'static str>,
}

/// Every keyword, in the canonical order the serializer writes terms in.
pub const KEYWORDS: &[KeywordSpec] = &[
    KeywordSpec {
        keyword: "type",
        syntax: "type:<type>[,<type>]",
        description: "Conventional type, `other` for none.",
        example: Some("type:fix,feat"),
    },
    KeywordSpec {
        keyword: "scope",
        syntax: "scope:<scope>[,<scope>]",
        description: "Conventional scope, `none` for none.",
        example: Some("scope:web"),
    },
    KeywordSpec {
        keyword: "author",
        syntax: "author:<who>[,<who>]",
        description: "E-mail in any case, or the exact name.",
        example: Some("author:\"Ann Lee\""),
    },
    KeywordSpec {
        keyword: "path",
        syntax: "path:<prefix>[,<prefix>]",
        description: "Touches a file under the prefix.",
        example: Some("path:crates/web"),
    },
    KeywordSpec {
        keyword: "after",
        syntax: "after:<period|YYYY-MM-DD>",
        description: "Within the period, or from the date.",
        example: Some("after:3m"),
    },
    KeywordSpec {
        keyword: "before",
        syntax: "before:<period|YYYY-MM-DD>",
        description: "Before the period, or the date.",
        example: Some("before:2026-01-31"),
    },
    KeywordSpec {
        keyword: "merge",
        syntax: "merge",
        description: "Merge commits.",
        example: Some("!merge"),
    },
    KeywordSpec {
        keyword: "<words>",
        syntax: "<word> or \"<words>\"",
        description: "The message has it, any case.",
        example: Some("\"fix login\" crash"),
    },
    KeywordSpec {
        keyword: "!",
        syntax: "!<term>",
        description: "The term does not match.",
        example: None,
    },
    KeywordSpec {
        keyword: "AND",
        syntax: "<term> <term>",
        description: "All terms match, `a,b` is either.",
        example: None,
    },
];

/// Keywords that read as unsupported rather than as message text, with the reason.
pub(crate) const UNSUPPORTED: &[(&str, &str)] =
    &[("committer", "`committer:` is not supported: the index keeps authors only. Use `author:`")];

/// The grammar as one line per keyword, ``- `syntax`: description``, for the docs and the MCP
/// tool descriptions, so the model reads the registry and nothing is written twice.
#[must_use]
pub fn grammar() -> String {
    KEYWORDS
        .iter()
        .map(|k| format!("- `{}`: {}", k.syntax, k.description))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The grammar on one line, `syntax: description` per keyword, for a tool argument the model
/// reads in a schema.
#[must_use]
pub fn grammar_line() -> String {
    KEYWORDS
        .iter()
        .map(|k| format!("`{}` {}", k.syntax, k.description))
        .collect::<Vec<_>>()
        .join(" ")
}
