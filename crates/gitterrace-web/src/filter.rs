//! The shared insights filter: commit type, scope, author, path prefix and period, parsed from
//! the query string and applied to the records before any aggregate, so every number on a
//! page respects every active chip.

use gitterrace_index::feed::{CommitRange, InsightsSource};
use gitterrace_index::query::{Leaf, Query, QueryError, Term, When};
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
    fn leaf(self, v: &str) -> Leaf {
        let v = vec![v.to_owned()];
        match self {
            Self::Kind => Leaf::Kind(v),
            Self::Scope => Leaf::Scope(v),
            Self::Author => Leaf::Author(v),
            Self::Path => Leaf::Path(v),
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

    /// This filter with the `d` term set to `value` (or removed with `None`).
    pub(crate) fn with(&self, d: Dim, value: Option<&str>) -> Self {
        self.swap(
            |l| d.is(l),
            value.map(|v| Term {
                neg: false,
                leaf: d.leaf(v),
            }),
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
mod tests {
    use std::sync::Arc;

    use gitterrace_index::feed::MemorySource;
    use gitterrace_index::{CommitRecord, FileStat};

    use super::{Dim, Filter, Period, under};
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

    /// The commits a filter's selection keeps, as a scoped source.
    fn kept(f: &Filter, all: &Src) -> Src {
        let m = gitterrace_index::query::Matcher::new(f.query_terms());
        Arc::new(gitterrace_index::Scoped::new(all, &m, None))
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

    fn ids(f: &Filter) -> Vec<String> {
        recs_of(&kept(f, &sample()))
            .iter()
            .map(|r| r.id.clone())
            .collect()
    }

    #[test]
    fn parses_and_ignores_empty() {
        let f = parse("type=fix&scope=&path=crates%2Fweb&bogus=1");
        assert_eq!(
            (f.kind(), f.scope(), f.path()),
            (Some("fix"), None, Some("crates/web"))
        );
    }

    /// Every older link form (`?type= ?scope= ?author= ?path= ?period=`) still selects the
    /// commits its `?q=` form selects, and writes links in that form.
    #[test]
    fn old_link_forms_select_what_q_selects() {
        // (old link, its `q` form, the canonical text both read as)
        for (old, q, text) in [
            ("type=fix", "type:fix", "type:fix"),
            ("scope=none", "scope:none", "scope:none"),
            ("author=ann", "author:ann", "author:ann"),
            (
                "path=%2Fcrates%2Fweb%2F",
                "path:crates/web",
                "path:crates/web",
            ),
            ("period=3m", "period:3m", "after:3m"),
            ("period=all", "period:all", ""),
            (
                "type=fix&author=ann&period=1y",
                "type:fix author:ann after:1y",
                "type:fix author:ann after:1y",
            ),
        ] {
            let (a, b) = (parse(old), parse(&format!("q={}", crate::fmt::enc_seg(q))));
            assert_eq!(a.query_terms().to_string(), text, "{old}");
            assert_eq!(a, b, "{old}");
            assert_eq!(ids(&a), ids(&b), "{old}");
        }
        assert_eq!(parse("period=3m").period_value(), Some(Period::Quarter));
        // `all` overrides a configured default period, so it survives into every link.
        assert_eq!(parse("period=all").period_value(), Some(Period::All));
        assert_eq!(
            parse("period=all&type=fix").query(&[]),
            "?q=type%3Afix&period=all"
        );
    }

    #[test]
    fn query_round_trips_and_encodes() {
        let f = parse("type=fix&author=Ann%20B&period=3m");
        assert_eq!(
            f.query(&[]),
            "?q=type%3Afix%20author%3A%22Ann%20B%22%20after%3A3m"
        );
        assert_eq!(parse(&f.query(&[])[1..]), f);
        assert_eq!(Filter::default().query(&[]), "");
        assert_eq!(
            f.with(Dim::Kind, None).query(&[("page", "2")]),
            "?q=author%3A%22Ann%20B%22%20after%3A3m&page=2"
        );
    }

    #[test]
    fn a_refused_query_keeps_its_text_and_the_rest() {
        let f = parse("q=a%20%7C%20b&type=fix");
        assert_eq!(f.text(), "a | b");
        assert!(f.error().is_some_and(|e| e.message.contains("Supported")));
        assert_eq!(f.kind(), Some("fix"));
    }

    #[test]
    fn filters_stack() {
        let q = |s: &str| Filter::parse(s);
        assert_eq!(ids(&Filter::default()), ["a", "b", "c", "d"]);
        assert_eq!(ids(&q("type:fix")), ["a", "d"]);
        assert_eq!(ids(&q("type:fix scope:web")), ["a"]);
        assert_eq!(ids(&q("type:other")), ["c"]);
        assert_eq!(ids(&q("scope:none")), ["b", "c"]);
        assert_eq!(ids(&q("author:ann type:fix")), ["a", "d"]);
        assert_eq!(ids(&q("type:nothing")), Vec::<String>::new());
        assert_eq!(ids(&q("!author:ann")), ["b"]);
    }

    #[test]
    fn path_filter_keeps_only_files_under_prefix() {
        let out = recs_of(&kept(&parse("path=crates/web"), &sample()));
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
    fn the_bar_never_builds_a_period_chip() {
        // The segmented control shows the period and All resets it: a chip would repeat it and
        // push the control sideways each time a period is chosen.
        for q in ["period=1w", "period=all&type=fix", "period=1y&path=src"] {
            let bar = parse(q).bar("/stats", &MemorySource::new(&[]), Period::Week);
            assert!(bar.chips.iter().all(|c| c.dim != "after"), "{q}");
        }
    }

    #[test]
    fn bar_chips_remove_one_filter() {
        let bar =
            parse("type=fix&scope=web").bar("/stats/types", &MemorySource::new(&[]), Period::Month);
        assert_eq!(bar.chips.len(), 2);
        assert_eq!(bar.chips[0].href, "/stats/types?q=scope%3Aweb");
        assert_eq!(bar.chips[1].href, "/stats/types?q=type%3Afix");
        // With no period asked for, the one in force is marked: the select never shows another.
        assert_eq!(bar.period.choices.iter().filter(|p| p.on).count(), 1);
        let neg = Filter::parse("!path:docs").chips("/log/HEAD", None);
        assert_eq!((neg[0].dim, neg[0].label.as_str()), ("path", "not docs"));
    }
}
