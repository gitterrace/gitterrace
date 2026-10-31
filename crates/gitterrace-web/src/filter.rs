//! The shared insights filter: commit type, scope, author, path prefix and period, parsed from
//! the query string and applied to the records before any aggregate, so every number on a
//! page respects every active chip.

use std::borrow::Cow;

use gitterrace_index::{CommitRecord, Period};
use serde::Deserialize;

use crate::fmt::{enc, esc, href};

/// Value that selects commits without a type (`other`) or without a scope (`none`).
pub(crate) const OTHER: &str = "other";
pub(crate) const NO_SCOPE: &str = "none";

const PERIODS: [(&str, &str); 5] = [
    ("1w", "Week"),
    ("1m", "Month"),
    ("3m", "Quarter"),
    ("1y", "Year"),
    ("all", "All"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dim {
    Kind,
    Scope,
    Author,
    Path,
    Period,
}

const DIMS: [Dim; 5] = [Dim::Kind, Dim::Scope, Dim::Author, Dim::Path, Dim::Period];

impl Dim {
    fn key(self) -> &'static str {
        match self {
            Self::Kind => "type",
            Self::Scope => "scope",
            Self::Author => "author",
            Self::Path => "path",
            Self::Period => "period",
        }
    }
}

/// Active filters. Empty strings count as unset.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub(crate) struct Filter {
    #[serde(rename = "type")]
    kind: Option<String>,
    scope: Option<String>,
    author: Option<String>,
    path: Option<String>,
    period: Option<String>,
}

pub(crate) struct Chip {
    pub(crate) dim: &'static str,
    pub(crate) label: String,
    pub(crate) href: String,
}

pub(crate) struct PeriodLink {
    pub(crate) label: &'static str,
    pub(crate) href: String,
    pub(crate) on: bool,
}

/// What `filterbar.html` renders: removable chips and the period picker.
pub(crate) struct FilterBar {
    pub(crate) chips: Vec<Chip>,
    pub(crate) periods: Vec<PeriodLink>,
    pub(crate) clear: String,
}

impl Filter {
    fn get(&self, d: Dim) -> Option<&str> {
        let v = match d {
            Dim::Kind => &self.kind,
            Dim::Scope => &self.scope,
            Dim::Author => &self.author,
            Dim::Path => &self.path,
            Dim::Period => &self.period,
        };
        v.as_deref().filter(|s| !s.is_empty())
    }

    pub(crate) fn kind(&self) -> Option<&str> {
        self.get(Dim::Kind)
    }

    pub(crate) fn scope(&self) -> Option<&str> {
        self.get(Dim::Scope)
    }

    /// Path prefix without surrounding slashes, `None` for the whole tree.
    pub(crate) fn author(&self) -> Option<&str> {
        self.get(Dim::Author)
    }

    pub(crate) fn path(&self) -> Option<&str> {
        self.get(Dim::Path)
            .map(|p| p.trim_matches('/'))
            .filter(|p| !p.is_empty())
    }

    pub(crate) fn period(&self) -> Option<Period> {
        self.get(Dim::Period)
            .and_then(Period::parse)
            .filter(|p| *p != Period::All)
    }

    /// The period including `All`, for pages that window themselves.
    pub(crate) fn period_value(&self) -> Option<Period> {
        self.get(Dim::Period).and_then(Period::parse)
    }

    pub(crate) fn is_active(&self) -> bool {
        DIMS.iter().any(|d| self.get(*d).is_some())
    }

    /// This filter with one dimension replaced (or removed with `None`).
    pub(crate) fn with(&self, d: Dim, value: Option<&str>) -> Self {
        let mut f = self.clone();
        let v = value.map(str::to_owned);
        match d {
            Dim::Kind => f.kind = v,
            Dim::Scope => f.scope = v,
            Dim::Author => f.author = v,
            Dim::Path => f.path = v,
            Dim::Period => f.period = v,
        }
        f
    }

    /// `?type=fix&scope=web`, or an empty string when nothing is set. `extra` pairs go last.
    pub(crate) fn query(&self, extra: &[(&str, &str)]) -> String {
        let mut parts: Vec<String> = DIMS
            .iter()
            .filter_map(|d| {
                self.get(*d)
                    .map(|v| format!("{}={}", d.key(), enc(v, false)))
            })
            .collect();
        parts.extend(extra.iter().map(|(k, v)| format!("{k}={}", enc(v, false))));
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

    fn matches(&self, r: &CommitRecord, window: (i64, i64)) -> bool {
        if r.time < window.0 || r.time >= window.1 {
            return false;
        }
        if let Some(k) = self.kind()
            && r.kind.as_deref().unwrap_or(OTHER) != k
        {
            return false;
        }
        if let Some(s) = self.scope()
            && r.scope.as_deref().unwrap_or(NO_SCOPE) != s
        {
            return false;
        }
        if let Some(a) = self.get(Dim::Author)
            && !r.email.eq_ignore_ascii_case(a)
            && r.author != a
        {
            return false;
        }
        true
    }

    /// The records the filters keep. With a path filter each kept commit lists only the files
    /// under the path, so line counts and churn are the path's own. Unfiltered input is borrowed.
    pub(crate) fn apply<'a>(&self, records: &'a [CommitRecord]) -> Cow<'a, [CommitRecord]> {
        if !self.is_active() {
            return Cow::Borrowed(records);
        }
        let newest = records.iter().map(|r| r.time).max().unwrap_or(0);
        let window = self
            .period()
            .map_or((i64::MIN, i64::MAX), |p| p.window(newest));
        let path = self.path();
        let out = records
            .iter()
            .filter(|r| self.matches(r, window))
            .filter_map(|r| match path {
                None => Some(r.clone()),
                Some(p) => {
                    let files: Vec<_> = r
                        .files
                        .iter()
                        .filter(|f| under(&f.path, p))
                        .cloned()
                        .collect();
                    (!files.is_empty()).then(|| CommitRecord { files, ..r.clone() })
                }
            })
            .collect();
        Cow::Owned(out)
    }

    /// Chips and period picker for a page served at `base`.
    pub(crate) fn bar(&self, base: &str, records: &[CommitRecord]) -> FilterBar {
        let chips = DIMS
            .iter()
            .filter_map(|d| {
                let v = self.get(*d)?;
                let label = match d {
                    Dim::Period => match v {
                        "1w" => "last week".to_owned(),
                        "1m" => "last month".to_owned(),
                        "3m" => "last 3 months".to_owned(),
                        "1y" => "last year".to_owned(),
                        _ => "all time".to_owned(),
                    },
                    Dim::Path => self.path()?.to_owned(),
                    Dim::Author => records
                        .iter()
                        .find(|r| r.email.eq_ignore_ascii_case(v))
                        .map_or_else(|| v.to_owned(), |r| r.author.clone()),
                    _ => v.to_owned(),
                };
                Some(Chip {
                    dim: d.key(),
                    label: esc(&label),
                    href: href(&format!("{base}{}", self.with(*d, None).query(&[]))),
                })
            })
            .collect();
        let periods = PERIODS
            .iter()
            .map(|(v, label)| PeriodLink {
                label,
                href: href(&format!(
                    "{base}{}",
                    self.with(Dim::Period, Some(v)).query(&[])
                )),
                on: self.get(Dim::Period) == Some(v),
            })
            .collect();
        FilterBar {
            chips,
            periods,
            clear: href(base),
        }
    }
}

/// True when `path` is `prefix` itself or inside it.
pub(crate) fn under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use gitterrace_index::{CommitRecord, FileStat};

    use super::{Dim, Filter, under};

    fn parse(q: &str) -> Filter {
        serde_urlencoded::from_str(q).expect("parse")
    }

    fn rec(
        id: &str,
        who: &str,
        day: i64,
        kind: Option<&str>,
        scope: Option<&str>,
        path: &str,
    ) -> CommitRecord {
        CommitRecord {
            id: id.to_owned(),
            author: who.to_owned(),
            time: day * 86_400,
            kind: kind.map(str::to_owned),
            scope: scope.map(str::to_owned),
            files: vec![
                FileStat {
                    path: path.to_owned(),
                    additions: 3,
                    ..FileStat::default()
                },
                FileStat {
                    path: "other/x.rs".to_owned(),
                    additions: 1,
                    ..FileStat::default()
                },
            ],
            ..CommitRecord::default()
        }
    }

    fn sample() -> Vec<CommitRecord> {
        vec![
            rec("a", "ann", 1, Some("fix"), Some("web"), "crates/web/a.rs"),
            rec("b", "bob", 50, Some("feat"), None, "crates/git/b.rs"),
            rec("c", "ann", 99, None, None, "crates/web/c.rs"),
            rec(
                "d",
                "ann",
                100,
                Some("fix"),
                Some("git"),
                "crates/webby/d.rs",
            ),
        ]
    }

    #[test]
    fn parses_and_ignores_empty() {
        let f = parse("type=fix&scope=&path=crates%2Fweb&bogus=1");
        assert_eq!(
            (f.kind(), f.scope(), f.path()),
            (Some("fix"), None, Some("crates/web"))
        );
        assert!(!Filter::default().is_active());
        assert!(f.is_active());
    }

    #[test]
    fn query_round_trips_and_encodes() {
        let f = parse("type=fix&author=Ann%20B&period=3m");
        assert_eq!(f.query(&[]), "?type=fix&author=Ann%20B&period=3m");
        assert_eq!(parse(&f.query(&[])[1..]), f);
        assert_eq!(Filter::default().query(&[]), "");
        assert_eq!(
            f.with(Dim::Kind, None).query(&[("page", "2")]),
            "?author=Ann%20B&period=3m&page=2"
        );
    }

    #[test]
    fn filters_stack() {
        let recs = sample();
        let ids = |f: &Filter| {
            f.apply(&recs)
                .iter()
                .map(|r| r.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&Filter::default()), ["a", "b", "c", "d"]);
        assert_eq!(ids(&parse("type=fix")), ["a", "d"]);
        assert_eq!(ids(&parse("type=fix&scope=web")), ["a"]);
        assert_eq!(ids(&parse("type=other")), ["c"]);
        assert_eq!(ids(&parse("scope=none")), ["b", "c"]);
        assert_eq!(ids(&parse("author=ann&type=fix")), ["a", "d"]);
        assert_eq!(ids(&parse("type=nothing")), Vec::<String>::new());
    }

    #[test]
    fn period_is_relative_to_newest_commit() {
        let recs = sample();
        let n = |q: &str| parse(q).apply(&recs).len();
        assert_eq!(n("period=1w"), 2);
        assert_eq!(n("period=3m"), 3);
        assert_eq!(n("period=1y"), 4);
        assert_eq!(n("period=all"), 4);
    }

    #[test]
    fn path_filter_keeps_only_files_under_prefix() {
        let recs = sample();
        let out = parse("path=crates/web").apply(&recs).into_owned();
        assert_eq!(
            out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["a", "c"]
        );
        assert!(out.iter().all(|r| r.files.len() == 1));
        assert!(
            under("crates/web/a.rs", "crates/web") && !under("crates/webby/d.rs", "crates/web")
        );
    }

    #[test]
    fn bar_chips_remove_one_filter() {
        let bar = parse("type=fix&scope=web").bar("/stats/types", &[]);
        assert_eq!(bar.chips.len(), 2);
        assert_eq!(bar.chips[0].href, "/stats/types?scope=web");
        assert_eq!(bar.chips[1].href, "/stats/types?type=fix");
        assert!(bar.periods.iter().all(|p| !p.on));
    }
}
