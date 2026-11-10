//! The shared insights filter: commit type, scope, author, path prefix and period, parsed from
//! the query string and applied to the records before any aggregate, so every number on a
//! page respects every active chip.

use gitterrace_index::feed::{CommitRange, InsightsSource};
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

    /// The period including `All`, for pages that window themselves.
    pub(crate) fn period_value(&self) -> Option<Period> {
        self.get(Dim::Period).and_then(|s| s.parse().ok())
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
            q: None,
        }
    }

    /// Chips and period picker for a page served at `base`, with `period` the one in force.
    pub(crate) fn bar(&self, base: &str, src: &dyn InsightsSource, period: Period) -> FilterBar {
        let chips: Vec<Chip> = DIMS
            .iter()
            // The period has its own control, which shows it and resets it: never a chip.
            .filter(|d| **d != Dim::Period)
            .filter_map(|d| {
                let v = self.get(*d)?;
                let label = match d {
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
        let choices = <Period as StringEnum>::ALL
            .iter()
            .map(|p| {
                Choice::new(
                    p.label(),
                    href(&format!(
                        "{base}{}",
                        self.with(Dim::Period, Some(p.as_str())).query(&[])
                    )),
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
    use gitterrace_index::feed::MemorySource;

    use super::{Dim, Filter, Period};

    fn parse(q: &str) -> Filter {
        let uri: axum::http::Uri = format!("/?{q}").parse().expect("uri");
        axum::extract::Query::try_from_uri(&uri).expect("parse").0
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

    /// The query parses to the index selection, which owns stacking and the path rule (the
    /// select tests): every dimension lands in its field, an empty one stays unset.
    #[test]
    fn the_query_becomes_the_selection() {
        let s = parse("type=fix&scope=web&author=ann&path=crates%2Fweb").selection();
        let want = gitterrace_index::Selection {
            kind: Some("fix".into()),
            scope: Some("web".into()),
            author: Some("ann".into()),
            path: Some("crates/web".into()),
            q: None,
        };
        assert_eq!(s, want);
        assert_eq!(
            parse("type=&path=").selection(),
            gitterrace_index::Selection::default()
        );
    }

    #[test]
    fn the_bar_never_builds_a_period_chip() {
        // The segmented control shows the period and All resets it: a chip would repeat it and
        // push the control sideways each time a period is chosen.
        for q in ["period=1w", "period=all&type=fix", "period=1y&path=src"] {
            let bar = parse(q).bar("/stats", &MemorySource::new(&[]), Period::Week);
            assert!(bar.chips.iter().all(|c| c.dim != "period"), "{q}");
        }
    }

    #[test]
    fn bar_chips_remove_one_filter() {
        let bar =
            parse("type=fix&scope=web").bar("/stats/types", &MemorySource::new(&[]), Period::Month);
        assert_eq!(bar.chips.len(), 2);
        assert_eq!(bar.chips[0].href, "/stats/types?scope=web");
        assert_eq!(bar.chips[1].href, "/stats/types?type=fix");
        // With no period asked for, the one in force is marked: the select never shows another.
        assert_eq!(bar.period.choices.iter().filter(|p| p.on).count(), 1);
    }
}
