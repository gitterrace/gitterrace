//! A [`Query`] compiled for testing commits: the one rule every surface applies.
//!
//! The index answers type, scope, author, merge and the date bounds from a commit's columns,
//! and the path terms from its file rows. Message words need the message: [`Matcher::words`]
//! hands them to the caller that has it (the log); statistics refuse them.

use crate::feed::{CommitRow, InsightsSource, LabelId};
use crate::query::{Leaf, Query, When};
use crate::{Selection, under};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Matcher {
    kinds: Vec<(bool, Vec<String>)>,
    scopes: Vec<(bool, Vec<String>)>,
    authors: Vec<(bool, Vec<String>)>,
    paths: Vec<(bool, Vec<String>)>,
    after: Vec<(bool, When)>,
    before: Vec<(bool, When)>,
    merge: Vec<bool>,
    words: Vec<(bool, String)>,
}

/// The columns of one commit a [`Matcher`] reads.
#[derive(Debug, Clone, Copy)]
pub struct Commit<'a> {
    pub kind: Option<&'a str>,
    pub scope: Option<&'a str>,
    pub name: &'a str,
    pub email: &'a str,
    pub time: i64,
    pub merge: bool,
}

impl Matcher {
    #[must_use]
    pub fn new(q: &Query) -> Self {
        let mut m = Self::default();
        for t in q.terms() {
            let n = t.neg;
            match &t.leaf {
                Leaf::Kind(v) => m.kinds.push((n, v.clone())),
                Leaf::Scope(v) => m.scopes.push((n, v.clone())),
                Leaf::Author(v) => m.authors.push((n, v.clone())),
                Leaf::Path(v) => m.paths.push((n, v.clone())),
                Leaf::After(w) => m.after.push((n, *w)),
                Leaf::Before(w) => m.before.push((n, *w)),
                Leaf::Merge => m.merge.push(n),
                Leaf::Text(s) => m.words.push((n, s.to_lowercase())),
            }
        }
        m
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether a path term trims each commit to some of its file rows.
    #[must_use]
    pub const fn has_path(&self) -> bool {
        !self.paths.is_empty()
    }

    /// The items of the first author term when it is positive: where an indexed answer
    /// starts, from the author postings.
    #[must_use]
    pub fn authors(&self) -> Option<&[String]> {
        self.authors.iter().find(|(n, _)| !n).map(|(_, v)| v.as_slice())
    }

    /// The one path prefix when the path terms are exactly one positive term of one item:
    /// what a git walk can filter by itself.
    #[must_use]
    pub fn single_path(&self) -> Option<&str> {
        match self.paths.as_slice() {
            [(false, v)] if v.len() == 1 => v.first().map(String::as_str),
            _ => None,
        }
    }

    /// The message words and phrases (lowercase), each with whether it is negated.
    #[must_use]
    pub fn words(&self) -> &[(bool, String)] {
        &self.words
    }

    /// The commit's columns pass every term but the path and message ones. `now` is the newest
    /// commit's time, which a period bound counts back from.
    #[must_use]
    pub fn commit(&self, c: &Commit<'_>, now: i64) -> bool {
        let label = |v: &[String], have: Option<&str>, none: &str| {
            let have = have.unwrap_or(none);
            v.iter().any(|w| w == have)
        };
        let all = |terms: &[(bool, Vec<String>)], hit: &dyn Fn(&[String]) -> bool| {
            terms.iter().all(|(n, v)| hit(v) != *n)
        };
        all(&self.kinds, &|v| label(v, c.kind, Selection::OTHER))
            && all(&self.scopes, &|v| label(v, c.scope, Selection::NO_SCOPE))
            && all(&self.authors, &|v| {
                v.iter().any(|w| by(w, c.name, c.email))
            })
            && self
                .after
                .iter()
                .all(|(n, w)| (c.time >= w.instant(now)) != *n)
            && self
                .before
                .iter()
                .all(|(n, w)| (c.time < w.instant(now)) != *n)
            && self.merge.iter().all(|n| c.merge != *n)
            // A path term keeps file rows, and a merge has none of its own.
            && !(c.merge && self.has_path())
    }

    /// [`Matcher::commit`] over an index row, its labels and identity read from `src`.
    #[must_use]
    pub fn commit_row(&self, src: &dyn InsightsSource, c: &CommitRow, now: i64) -> bool {
        let label = |id: Option<LabelId>| id.and_then(|l| src.label(l));
        let (kind, scope) = (label(c.kind), label(c.scope));
        let (name, email) = src.ident(c.ident).unwrap_or_default();
        self.commit(
            &Commit {
                kind: kind.as_deref(),
                scope: scope.as_deref(),
                name: &name,
                email: &email,
                time: c.time,
                merge: c.merge,
            },
            now,
        )
    }

    /// Whether a file row at `path` is kept: under every positive path term (any of its
    /// items) and under no negated one.
    #[must_use]
    pub fn row(&self, path: &str) -> bool {
        self.paths.iter().all(|(n, v)| v.iter().any(|p| under(path, p)) != *n)
    }

    /// Whether the message (any case) passes every word term.
    #[must_use]
    pub fn message(&self, text: &str) -> bool {
        if self.words.is_empty() {
            return true;
        }
        let text = text.to_lowercase();
        self.words.iter().all(|(n, w)| text.contains(w.as_str()) != *n)
    }

    /// The records the query keeps, a path term trimming each to its files under it. `now`
    /// as in [`Matcher::commit`]. The reference the indexed views are tested against.
    #[must_use]
    pub fn apply(&self, records: &[crate::CommitRecord], now: i64) -> Vec<crate::CommitRecord> {
        records
            .iter()
            .filter(|r| {
                self.commit(
                    &Commit {
                        kind: r.kind.as_deref(),
                        scope: r.scope.as_deref(),
                        name: &r.author,
                        email: &r.email,
                        time: r.time,
                        merge: r.merge,
                    },
                    now,
                )
            })
            .filter_map(|r| {
                if !self.has_path() {
                    return Some(r.clone());
                }
                let files: Vec<_> = r.files.iter().filter(|f| self.row(&f.path)).cloned().collect();
                (!files.is_empty()).then(|| crate::CommitRecord { files, ..r.clone() })
            })
            .collect()
    }
}

/// Whether `who` (e-mail in any case, or the exact name) wrote the commit.
#[must_use]
pub fn by(who: &str, name: &str, email: &str) -> bool {
    email.eq_ignore_ascii_case(who) || name == who
}
