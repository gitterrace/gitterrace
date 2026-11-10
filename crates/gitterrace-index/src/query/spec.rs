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
}

/// Every keyword, in the canonical order the serializer writes terms in.
pub const KEYWORDS: &[KeywordSpec] = &[
    KeywordSpec {
        keyword: "type",
        syntax: "type:<type>[,<type>]",
        description: "Conventional-commit type of the subject. `other` matches commits without one.",
        example: Some("type:fix,feat"),
    },
    KeywordSpec {
        keyword: "scope",
        syntax: "scope:<scope>[,<scope>]",
        description: "Conventional-commit scope. `none` matches commits without one.",
        example: Some("scope:web"),
    },
    KeywordSpec {
        keyword: "author",
        syntax: "author:<name or email>[,...]",
        description: "Commits by this author: the e-mail in any case, or the exact name. Quote a name with spaces.",
        example: Some("author:\"Ann Lee\""),
    },
    KeywordSpec {
        keyword: "path",
        syntax: "path:<prefix>[,<prefix>]",
        description: "Commits touching a file under the path prefix (a directory or a file).",
        example: Some("path:crates/web"),
    },
    KeywordSpec {
        keyword: "after",
        syntax: "after:<1w|1m|3m|1y|YYYY-MM-DD>",
        description: "Committed within the period before the newest commit, or on or after the date.",
        example: Some("after:3m"),
    },
    KeywordSpec {
        keyword: "before",
        syntax: "before:<1w|1m|3m|1y|YYYY-MM-DD>",
        description: "Committed before the period started, or before the date.",
        example: Some("before:2026-01-31"),
    },
    KeywordSpec {
        keyword: "period",
        syntax: "period:<1w|1m|3m|1y|all>",
        description: "The insights period. Read as `after:`, `all` as no bound.",
        example: Some("period:1y"),
    },
    KeywordSpec {
        keyword: "merge",
        syntax: "merge",
        description: "Merge commits. `!merge` leaves them out.",
        example: Some("!merge"),
    },
    KeywordSpec {
        keyword: "<words>",
        syntax: "<word> or \"<phrase>\"",
        description: "Commits whose message contains the word or phrase, any case.",
        example: Some("\"fix login\" crash"),
    },
    KeywordSpec {
        keyword: "!",
        syntax: "!<term>",
        description: "Not: commits the term does not match.",
        example: None,
    },
    KeywordSpec {
        keyword: "AND",
        syntax: "<term> <term>",
        description: "Terms side by side must all match. A comma inside one value means any of them.",
        example: None,
    },
];

/// Keywords that read as unsupported rather than as message text, with the reason.
pub(crate) const UNSUPPORTED: &[(&str, &str)] = &[(
    "committer",
    "`committer:` is not supported: the index keeps authors only. Use `author:`",
)];
