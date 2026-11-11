//! The keyword table: what the parser accepts, what the docs list and what the filter field
//! offers as completion, from one list.

/// One documented keyword of the commit filter.
#[derive(Debug, Clone, Copy)]
pub struct KeywordSpec {
    /// The literal before the `:`, or the whole bare word for a flag such as `merge`.
    pub keyword: &'static str,
    /// The full form, `author:<name or email>`.
    pub syntax: &'static str,
    pub description: &'static str,
    /// A valid expression the parser must accept, `None` for the operator rows.
    pub example: Option<&'static str>,
    /// The one-key URL parameter older links write this keyword as (`?type=fix`), which
    /// [`fold`](crate::query::fold) reads into the term. `None` for keywords with no such form.
    pub param: Option<&'static str>,
}

/// Every keyword, in the canonical order the serializer writes terms in.
pub const KEYWORDS: &[KeywordSpec] = &[
    KeywordSpec {
        keyword: "type",
        param: Some("type"),
        syntax: "type:<type>[,<type>]",
        description: "Conventional type; `other` is none.",
        example: Some("type:fix,feat"),
    },
    KeywordSpec {
        keyword: "scope",
        param: Some("scope"),
        syntax: "scope:<scope>[,<scope>]",
        description: "Conventional scope; `none` is none.",
        example: Some("scope:web"),
    },
    KeywordSpec {
        keyword: "author",
        param: Some("author"),
        syntax: "author:<who>[,<who>]",
        description: "E-mail in any case, or the exact name.",
        example: Some("author:\"Ann Lee\""),
    },
    KeywordSpec {
        keyword: "path",
        param: Some("path"),
        syntax: "path:<prefix>[,<prefix>]",
        description: "Touches a file under the prefix.",
        example: Some("path:crates/web"),
    },
    KeywordSpec {
        keyword: "after",
        param: None,
        syntax: "after:<period|YYYY-MM-DD>",
        description: "Within the period, or from the date.",
        example: Some("after:3m"),
    },
    KeywordSpec {
        keyword: "before",
        param: None,
        syntax: "before:<period|YYYY-MM-DD>",
        description: "Before the period, or the date.",
        example: Some("before:2026-01-31"),
    },
    KeywordSpec {
        keyword: "period",
        param: Some("period"),
        syntax: "period:<1w|1m|3m|1y|all>",
        description: "Old form of `after:`; `all` is none.",
        example: Some("period:1y"),
    },
    KeywordSpec {
        keyword: "merge",
        param: None,
        syntax: "merge",
        description: "Merge commits.",
        example: Some("!merge"),
    },
    KeywordSpec {
        keyword: "<words>",
        param: None,
        syntax: "<word> or \"<words>\"",
        description: "The message has it, any case.",
        example: Some("\"fix login\" crash"),
    },
    KeywordSpec {
        keyword: "!",
        param: None,
        syntax: "!<term>",
        description: "The term does not match.",
        example: None,
    },
    KeywordSpec {
        keyword: "AND",
        param: None,
        syntax: "<term> <term>",
        description: "All terms match; `a,b` is either.",
        example: None,
    },
];

/// Keywords that read as unsupported rather than as message text, with the reason.
pub(crate) const UNSUPPORTED: &[(&str, &str)] = &[(
    "committer",
    "`committer:` is not supported: the index keeps authors only. Use `author:`",
)];

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

/// The older one-key URL parameters and the keyword each one reads as, `(param, keyword)`.
#[must_use]
pub fn aliases() -> Vec<(&'static str, &'static str)> {
    KEYWORDS
        .iter()
        .filter_map(|k| k.param.map(|p| (p, k.keyword)))
        .collect()
}
