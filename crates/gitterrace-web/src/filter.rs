//! The shared insights filter: commit type, scope, author, path prefix and period, parsed from
//! the query string and applied to the records before any aggregate, so every number on a
//! page respects every active chip.

use gitterrace_index::feed::{CommitRange, InsightsSource};
use gitterrace_index::query::{Leaf, Query, QueryError, Term, When, fold};
use gitterrace_index::{Period, Selection};
use gitterrace_types::StringEnum;
use serde::Deserialize;

use askama::Template;

use crate::empty::Empty;
use crate::fmt::{enc_seg, href};
use crate::insights::OptLink;
use crate::seg::{Choice, Seg};

/// Value that selects commits without a scope (`none`).
pub(crate) const NO_SCOPE: &str = Selection::NO_SCOPE;
pub(crate) use gitterrace_index::under;

/// The filter of a page: the URL's `q` and the one-key forms older links use (`type`,
/// `scope`, `author`, `path`, `period`), read into one [`Query`]. Every link it writes says
/// it as `?q=` alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Filter {
    query: Query,
    /// What `q` said when it did not parse: the field shows it again with the error.
    refused: Option<(String, QueryError)>,
    /// `period=all`: the whole history, over a configured default period.
    all: bool,
}

/// The URL keys, as written.
#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(flatten)]
    select: Selection,
    period: Option<String>,
}

impl<'de> Deserialize<'de> for Filter {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = Raw::deserialize(d)?;
        Ok(Self::of_raw(&raw))
    }
}

pub(crate) struct Chip {
    pub(crate) dim: &'static str,
    pub(crate) label: String,
    pub(crate) href: String,
}

/// The chips of a page that has no period bar (the log), shaped as `_chips.html` reads them.
pub(crate) struct ChipRow {
    pub(crate) chips: Vec<Chip>,
    pub(crate) clear: String,
}

/// The insights filters: removable chips and the page head's toolbar (`toolbar`), which holds
/// the period, the page's own choice and its "On this page" menu.
pub(crate) struct FilterBar {
    pub(crate) chips: Vec<Chip>,
    pub(crate) period: Seg,
    /// A second choice of the page (the contributors metric), after the period.
    pub(crate) extra: Option<Seg>,
    /// "On this page" (`markdown::outline_menu`), at the row's end; empty for none.
    pub(crate) jump: String,
    pub(crate) clear: String,
    /// What the page shows when the filters match no commit, with the way back.
    pub(crate) nothing: Empty,
}

/// The terms a link sets with [`Filter::link`]: one keyword and its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dim {
    Kind,
    Scope,
    Author,
    Path,
}

impl Dim {
    /// The URL key of the term, as [`fold`] reads it.
    const fn key(self) -> &'static str {
        match self {
            Self::Kind => "type",
            Self::Scope => "scope",
            Self::Author => "author",
            Self::Path => "path",
        }
    }

    const fn is(self, l: &Leaf) -> bool {
        matches!(
            (self, l),
            (Self::Kind, Leaf::Kind(_))
                | (Self::Scope, Leaf::Scope(_))
                | (Self::Author, Leaf::Author(_))
                | (Self::Path, Leaf::Path(_))
        )
    }
}

impl Filter {
    fn of_raw(raw: &Raw) -> Self {
        let period = raw
            .period
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| format!("period:{p}"));
        let with_period = Selection {
            q: Some(format!(
                "{} {}",
                period.unwrap_or_default(),
                raw.select.q.as_deref().unwrap_or_default()
            )),
            ..raw.select.clone()
        };
        // `all` bounds nothing, so the query drops it: remembered here, it still overrides a
        // configured default period, whether `?period=all` or `period:all` in the text said it.
        let all = raw.period.as_deref().map(str::trim) == Some(Period::All.as_str())
            || raw
                .select
                .q
                .as_deref()
                .is_some_and(|t| t.split_whitespace().any(|w| w == "period:all"));
        match with_period.query() {
            Ok(query) => Self {
                query,
                refused: None,
                all,
            },
            Err(e) => {
                // The one-key forms and the period still hold; only the text is shown back.
                let rest = Selection {
                    q: None,
                    ..raw.select.clone()
                };
                Self {
                    query: rest.query().unwrap_or_default(),
                    refused: Some((raw.select.q.clone().unwrap_or_default(), e)),
                    all,
                }
            }
        }
    }

    /// The filter of `text`, as a `?q=` link to it reads.
    #[cfg(test)]
    pub(crate) fn parse(text: &str) -> Self {
        Self::of_raw(&Raw {
            select: Selection {
                q: Some(text.to_owned()),
                ..Selection::default()
            },
            period: None,
        })
    }

    pub(crate) const fn query_terms(&self) -> &Query {
        &self.query
    }

    /// The text the field shows: what was refused, else the canonical text.
    pub(crate) fn text(&self) -> String {
        self.refused
            .as_ref()
            .map_or_else(|| self.query.to_string(), |(t, _)| t.clone())
    }

    pub(crate) fn error(&self) -> Option<&QueryError> {
        self.refused.as_ref().map(|(_, e)| e)
    }

    fn one(&self, d: Dim) -> Option<&str> {
        self.query.terms().iter().find_map(|t| match &t.leaf {
            l if !t.neg && d.is(l) => match l {
                Leaf::Kind(v) | Leaf::Scope(v) | Leaf::Author(v) | Leaf::Path(v)
                    if v.len() == 1 =>
                {
                    v.first().map(String::as_str)
                }
                _ => None,
            },
            _ => None,
        })
    }

    /// The single type the filter selects, when it selects exactly one.
    pub(crate) fn kind(&self) -> Option<&str> {
        self.one(Dim::Kind)
    }

    pub(crate) fn scope(&self) -> Option<&str> {
        self.one(Dim::Scope)
    }

    pub(crate) fn author(&self) -> Option<&str> {
        self.one(Dim::Author)
    }

    /// The single path prefix, `None` for the whole tree.
    pub(crate) fn path(&self) -> Option<&str> {
        self.one(Dim::Path)
    }

    /// The period of an `after:` period term, for pages that window themselves.
    pub(crate) fn period_value(&self) -> Option<Period> {
        let term = self.query.terms().iter().find_map(|t| match t.leaf {
            Leaf::After(When::Period(p)) if !t.neg => Some(p),
            _ => None,
        });
        term.or_else(|| self.all.then_some(Period::All))
    }

    /// This filter with the `d` term set to `value` (or removed with `None` or an empty or
    /// root value, as the whole-repository row links it).
    pub(crate) fn with(&self, d: Dim, value: Option<&str>) -> Self {
        let value = value
            .map(|v| {
                if d == Dim::Path {
                    v.trim_matches('/')
                } else {
                    v.trim()
                }
            })
            .filter(|v| !v.is_empty());
        self.swap(
            |l| d.is(l),
            value.and_then(|v| fold(d.key(), v).ok().flatten()),
        )
    }

    /// This filter with the period set; `All` removes it.
    pub(crate) fn with_period(&self, p: Period) -> Self {
        Self {
            all: p == Period::All,
            ..self.swap(
                |l| matches!(l, Leaf::After(When::Period(_))),
                (p != Period::All).then_some(Term {
                    neg: false,
                    leaf: Leaf::After(When::Period(p)),
                }),
            )
        }
    }

    fn swap(&self, drop: impl Fn(&Leaf) -> bool, add: Option<Term>) -> Self {
        Self {
            query: self.query.replace(drop, add),
            refused: None,
            all: self.all,
        }
    }

    /// `?q=type%3Afix`, or an empty string when nothing is set. `extra` pairs go last.
    pub(crate) fn query(&self, extra: &[(&str, &str)]) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.query.is_empty() {
            parts.push(format!("q={}", enc_seg(&self.query.to_string())));
        }
        if self.all {
            parts.push(format!("period={}", Period::All.as_str()));
        }
        parts.extend(extra.iter().map(|(k, v)| format!("{k}={}", enc_seg(v))));
        if parts.is_empty() {
            String::new()
        } else {
            format!("?{}", parts.join("&"))
        }
    }

    /// Link to `base` with `d` set to `value` on top of the current filters.
    pub(crate) fn link(&self, base: &str, d: Dim, value: &str) -> String {
        href(&format!("{base}{}", self.with(d, Some(value)).query(&[])))
    }

    /// The selection the scope reads: the filter text without its period, which the scope
    /// applies as the page's period ([`Filter::period_value`]).
    pub(crate) fn selection(&self) -> Selection {
        Selection::of(
            &self
                .query
                .replace(|l| matches!(l, Leaf::After(When::Period(_))), None),
        )
    }

    /// One removable chip per term, each linking to `base` without it.
    pub(crate) fn chips(&self, base: &str, src: Option<&dyn InsightsSource>) -> Vec<Chip> {
        self.query
            .terms()
            .iter()
            .enumerate()
            // The period has its own control, which shows it and resets it: never a chip.
            .filter(|(_, t)| !matches!(t.leaf, Leaf::After(When::Period(_))) || t.neg)
            .map(|(i, t)| {
                let shown = Term {
                    neg: false,
                    leaf: t.leaf.clone(),
                }
                .to_string();
                let key = t.leaf.keyword();
                let value = shown
                    .strip_prefix(key)
                    .and_then(|v| v.strip_prefix(':'))
                    .unwrap_or(&shown);
                let label = match (&t.leaf, src) {
                    (Leaf::Author(v), Some(src)) if v.len() == 1 => author_name(src, &v[0]),
                    _ => value.trim_matches('"').to_owned(),
                };
                let label = if t.neg { format!("not {label}") } else { label };
                let dim = if key.is_empty() { "message" } else { key };
                let rest = Self {
                    query: self.query.without(i),
                    refused: None,
                    all: self.all,
                };
                Chip {
                    dim,
                    // Plain text: the template escapes it.
                    label,
                    href: href(&format!("{base}{}", rest.query(&[]))),
                }
            })
            .collect()
    }

    /// Chips and period picker for a page served at `base`, with `period` the one in force.
    pub(crate) fn bar(&self, base: &str, src: &dyn InsightsSource, period: Period) -> FilterBar {
        let chips = self.chips(base, Some(src));
        let choices = <Period as StringEnum>::ALL
            .iter()
            .map(|p| {
                Choice::new(
                    p.label(),
                    href(&format!("{base}{}", self.with_period(*p).query(&[]))),
                    *p == period,
                )
            })
            .collect();
        // The choices say what they are: the name stays for assistive tech only.
        let period = Seg::links("Period", choices).no_caption();
        let nothing = Empty::new("No commits match these filters.");
        // With no filter set the period is what is narrowing; its own picker is the next step.
        let nothing = if chips.is_empty() {
            nothing
        } else {
            nothing.step("Clear all filters", href(base))
        };
        FilterBar {
            chips,
            period,
            extra: None,
            jump: String::new(),
            clear: href(base),
            nothing,
        }
    }
}

/// The toolbar markup, from `toolbar.html`.
#[derive(Template)]
#[template(path = "toolbar.html")]
struct ToolbarMarkup<'a> {
    fb: &'a FilterBar,
    opts: &'a [OptLink],
}

impl FilterBar {
    /// The page head's toolbar: the period, the page's own choice, the options line and
    /// "On this page", as markup.
    pub(crate) fn toolbar(&self, opts: &[OptLink]) -> String {
        ToolbarMarkup { fb: self, opts }
            .render()
            .unwrap_or_default()
    }
}

/// Name of the oldest commit whose email is `email`, as the chip shows it, else `email`.
fn author_name(src: &dyn InsightsSource, email: &str) -> String {
    src.commits_in(CommitRange::All)
        .find_map(|c| {
            src.ident(c.ident)
                .filter(|(_, e)| e.eq_ignore_ascii_case(email))
                .map(|(n, _)| n.to_string())
        })
        .unwrap_or_else(|| email.to_owned())
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
