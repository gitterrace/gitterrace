//! The commit filter: one text query, `author:ann path:src !merge fix`, for every surface.
//!
//! The log, the insights pages and the API all read it. [`Query::parse`] reads it, `Display`
//! writes the canonical text back, and [`KEYWORDS`] lists every keyword for the docs and
//! completion. Terms keep the order written: canonical text is the user's terms, respaced and
//! requoted, so a chip or a link never reorders what was typed.

mod parse;
mod spec;

use gitterrace_types::Period;

pub use spec::{KEYWORDS, KeywordSpec};

/// A parsed filter: every term must match, in the order written.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<Term>,
}

/// One term, negated with `!`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub neg: bool,
    pub leaf: Leaf,
}

/// What one term tests. A list value matches when any of its items does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leaf {
    Kind(Vec<String>),
    Scope(Vec<String>),
    Author(Vec<String>),
    Path(Vec<String>),
    After(When),
    Before(When),
    Merge,
    /// A word or phrase of the message, any case.
    Text(String),
}

/// A date bound: a period back from the newest commit, or a calendar day (UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Period(Period),
    Day { y: i32, m: u8, d: u8 },
}

/// Text the parser refuses, with the byte range it points at.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct QueryError {
    pub start: usize,
    pub end: usize,
    pub message: String,
}

impl Query {
    #[must_use]
    pub const fn new(terms: Vec<Term>) -> Self {
        Self { terms }
    }

    /// Parses filter text. Empty text is the empty query.
    ///
    /// # Errors
    /// [`QueryError`] for an unknown operator, a missing or bad value, or an unclosed quote.
    pub fn parse(text: &str) -> Result<Self, QueryError> {
        parse::terms(text).map(Self::new)
    }

    #[must_use]
    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// This query with term `i` removed: what a chip links to.
    #[must_use]
    pub fn without(&self, i: usize) -> Self {
        let mut terms = self.terms.clone();
        if i < terms.len() {
            terms.remove(i);
        }
        Self { terms }
    }

    /// This query with the terms `drop` matches replaced by `add`, at the first one's place
    /// (else appended), so a picker that swaps a term leaves the order as written.
    #[must_use]
    pub fn replace(&self, drop: impl Fn(&Leaf) -> bool, add: Option<Term>) -> Self {
        let at = self.terms.iter().position(|t| drop(&t.leaf));
        let mut terms: Vec<Term> = self
            .terms
            .iter()
            .filter(|t| !drop(&t.leaf))
            .cloned()
            .collect();
        if let Some(t) = add {
            terms.insert(at.unwrap_or(terms.len()), t);
        }
        Self { terms }
    }
}

impl Leaf {
    /// The keyword this leaf is written with, `""` for message text.
    #[must_use]
    pub const fn keyword(&self) -> &'static str {
        match self {
            Self::Kind(_) => "type",
            Self::Scope(_) => "scope",
            Self::Author(_) => "author",
            Self::Path(_) => "path",
            Self::After(_) => "after",
            Self::Before(_) => "before",
            Self::Merge => "merge",
            Self::Text(_) => "",
        }
    }
}

impl When {
    /// The bound in epoch seconds with `now` the newest commit's time.
    #[must_use]
    pub fn instant(self, now: i64) -> i64 {
        match self {
            Self::Period(p) => p.window(now).0,
            Self::Day { y, m, d } => {
                crate::days_from_civil(i64::from(y), i64::from(m), i64::from(d)) * 86_400
            }
        }
    }
}

impl std::fmt::Display for When {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Period(p) => f.write_str(p.as_str()),
            Self::Day { y, m, d } => write!(f, "{y:04}-{m:02}-{d:02}"),
        }
    }
}

impl std::fmt::Display for Term {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.neg {
            f.write_str("!")?;
        }
        let key = self.leaf.keyword();
        match &self.leaf {
            Leaf::Kind(v) | Leaf::Scope(v) | Leaf::Author(v) | Leaf::Path(v) => {
                let items: Vec<String> = v.iter().map(|s| parse::quote(s)).collect();
                write!(f, "{key}:{}", items.join(","))
            }
            Leaf::After(w) | Leaf::Before(w) => write!(f, "{key}:{w}"),
            Leaf::Merge => f.write_str(key),
            Leaf::Text(s) => f.write_str(&parse::quote_text(s)),
        }
    }
}

/// The canonical text: the terms one space apart, values quoted only when they must be.
impl std::fmt::Display for Query {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, t) in self.terms.iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{t}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
