//! The shared insights filter: commit type, scope, author, path prefix and period, parsed from
//! the query string and applied to the records before any aggregate, so every number on a
//! page respects every active chip.

use std::sync::Arc;

use gitterrace_index::feed::{CommitRange, InsightsSource};
use gitterrace_index::{Period, Scoped, Selection};
use serde::Deserialize;

use crate::fmt::{enc_seg, href};
use crate::insights::Src;

/// Value that selects commits without a scope (`none`).
pub(crate) const NO_SCOPE: &str = Selection::NO_SCOPE;
pub(crate) use gitterrace_index::under;

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
    const fn key(self) -> &'static str {
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

/// What `_filterbar.html` renders: removable chips and the period picker.
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
            .filter_map(|d| self.get(*d).map(|v| format!("{}={}", d.key(), enc_seg(v))))
            .collect();
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

    /// The commit selection, without the period: what the API's `type`, `scope`, `author` and
    /// `path` parameters parse to as well.
    pub(crate) fn selection(&self) -> Selection {
        let own = |d| self.get(d).map(str::to_owned);
        Selection {
            kind: own(Dim::Kind),
            scope: own(Dim::Scope),
            author: own(Dim::Author),
            path: own(Dim::Path),
        }
    }

    /// The commits the filters keep, the period window included, measured from the whole
    /// source. With a path filter each kept commit lists only the files under the path.
    /// Unfiltered input comes back as the same source, so its shared tables still apply.
    pub(crate) fn apply(&self, all: &Src) -> Src {
        let sel = self.selection();
        let window = self.period().map(|p| {
            let (from, to) = p.window(gitterrace_index::feed::reference_time(&**all));
            from..to
        });
        if sel.is_empty() && window.is_none() {
            return Arc::clone(all);
        }
        Arc::new(Scoped::new(all, &sel, window.as_ref()))
    }

    /// Chips and period picker for a page served at `base`.
    pub(crate) fn bar(&self, base: &str, src: &dyn InsightsSource) -> FilterBar {
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
                    Dim::Author => author_name(src, v),
                    _ => v.to_owned(),
                };
                Some(Chip {
                    dim: d.key(),
                    // Plain text: the template escapes it.
                    label,
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
mod tests {
    use std::sync::Arc;

    use gitterrace_index::feed::MemorySource;
    use gitterrace_index::{CommitRecord, FileStat};

    use super::{Dim, Filter, under};
    use crate::insights::Src;

    fn parse(q: &str) -> Filter {
        let uri: axum::http::Uri = format!("/?{q}").parse().expect("uri");
        axum::extract::Query::try_from_uri(&uri).expect("parse").0
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
                    path: path.into(),
                    additions: 3,
                    ..FileStat::default()
                },
                FileStat {
                    path: "other/x.rs".into(),
                    additions: 1,
                    ..FileStat::default()
                },
            ],
            ..CommitRecord::default()
        }
    }

    fn sample() -> Src {
        Arc::new(MemorySource::shared(Arc::new(sample_records())))
    }

    /// What a filtered source holds: each commit id with its kept rows' paths.
    fn recs_of(s: &Src) -> Vec<CommitRecord> {
        use gitterrace_index::feed::CommitRange;
        let ranks: Vec<_> = s.commits_in(CommitRange::All).map(|c| c.rank).collect();
        s.rows_for(ranks)
            .map(|(rank, rows)| CommitRecord {
                id: s.commit_id(rank).as_deref().unwrap_or_default().to_owned(),
                files: rows
                    .iter()
                    .map(|r| gitterrace_index::FileStat {
                        path: s.path(r.path).unwrap_or_default(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            })
            .collect()
    }

    fn sample_records() -> Vec<CommitRecord> {
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
            recs_of(&f.apply(&recs))
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
        let out = recs_of(&parse("path=crates/web").apply(&recs));
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
        let bar = parse("type=fix&scope=web").bar("/stats/types", &MemorySource::new(&[]));
        assert_eq!(bar.chips.len(), 2);
        assert_eq!(bar.chips[0].href, "/stats/types?scope=web");
        assert_eq!(bar.chips[1].href, "/stats/types?type=fix");
        assert!(bar.periods.iter().all(|p| !p.on));
    }
}
