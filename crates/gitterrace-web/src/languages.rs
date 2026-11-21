//! Languages page: lines of code per language along the first-parent history, a brush that
//! selects a commit range, and the range card the server renders from `?from&to`.

pub(crate) mod chain;
mod render;
mod view;

pub(crate) use chain::{Skeleton, state_key};

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use gitterrace_index::{Period, Scope, ScopeParams};
use serde::Deserialize;

use crate::{
    error::Result,
    filter::Filter,
    fmt,
    insights::{cached_keyed, counted, insights},
    languages::render::{body, sample},
    pagehead::PageHead,
    routes::Shared,
    views::Chrome,
};

const TOP: usize = 7;
const W: f64 = 960.0;
const H: f64 = 280.0;
const LEFT: f64 = 72.0;
const RIGHT: f64 = 8.0;
const TOP_PAD: f64 = 10.0;
const BOTTOM: f64 = 26.0;
const SAMPLES: usize = 700;
const COMMIT_LIST: usize = 500;

/// Chain positions whose subject the page shows: the chart samples and the newest commits
/// of the range list. Everything else is never read, so it is never fetched.
fn shown(n: usize, view: View) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let keep: Vec<usize> = view.range.map_or(vec![], |(a, b)| vec![a, b]);
    let mut v = sample(n, &keep);
    let (from, to) = view.range.unwrap_or((0, n - 1));
    let shown = (to - from + 1).min(COMMIT_LIST);
    v.extend(to + 1 - shown..=to);
    v.sort_unstable();
    v.dedup();
    v
}

/// Subjects of the chain positions the page shows, fetched for those commits only.
struct Subjects(HashMap<usize, String>);

impl Subjects {
    fn of(&self, i: usize) -> String {
        self.0.get(&i).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LocQuery {
    x: Option<String>,
    scale: Option<String>,
    from: Option<usize>,
    to: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    Commits,
    /// The period holds no commit.
    Nothing,
}

/// View state carried by every link on the page.
#[derive(Clone, Copy)]
struct View {
    time: bool,
    share: bool,
    /// The positions the card covers: a dragged range, else the period's stretch.
    range: Option<(usize, usize)>,
    /// The range comes from `from`/`to` (a drag) and replaces the period.
    picked: bool,
    period: Period,
    dflt: Period,
    /// The period holds no commit.
    held: Held,
}

impl View {
    /// The view of `q`, with `period` the shared filter's (`?q=after:1w` or `?period=1w`).
    fn parse(q: &LocQuery, period: Option<Period>, n: usize, dflt: Period) -> Self {
        let range = match (q.from, q.to) {
            (None, None) => None,
            (f, t) => {
                let last = n.saturating_sub(1);
                let to = t.unwrap_or(last).min(last);
                let from = f.unwrap_or(0).min(to);
                Some((from, to))
            }
        };
        Self {
            time: q.x.as_deref() == Some("time"),
            share: q.scale.as_deref() == Some("share"),
            range,
            picked: range.is_some(),
            period: period.unwrap_or(dflt),
            dflt,
            held: Held::Commits,
        }
    }

    /// Whether the period still has to be turned into a stretch of the chain.
    const fn wants_stretch(self) -> bool {
        !self.picked && self.period.days().is_some()
    }

    /// Sets the period's stretch: from `first`, the oldest position dated in the period, to
    /// the newest commit of a chain of `n`.
    const fn with_stretch(mut self, first: Option<usize>, n: usize) -> Self {
        match first {
            Some(a) => self.range = Some((a, n - 1)),
            None => self.held = Held::Nothing,
        }
        self
    }

    fn href(self, f: impl FnOnce(&mut Self)) -> String {
        let mut v = self;
        f(&mut v);
        let mut extra: Vec<(&str, String)> = Vec::new();
        if v.time {
            extra.push(("x", "time".to_owned()));
        }
        if v.share {
            extra.push(("scale", "share".to_owned()));
        }
        // The period is said the way every insights page says it, through the shared filter.
        let mut filter = Filter::default();
        if v.picked {
            if let Some((a, b)) = v.range {
                extra.push(("from", a.to_string()));
                extra.push(("to", b.to_string()));
            }
        } else if v.period != v.dflt {
            filter = filter.with_period(v.period);
        }
        let extra: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
        fmt::href(&format!("/stats/languages{}", filter.query(&extra)))
    }
}

#[derive(Template)]
#[template(path = "languages.html")]
struct LanguagesPage {
    c: Chrome,
    head: PageHead,
    body: view::Body,
}

pub(crate) async fn page(
    State(app): Shared,
    Query(q): Query<LocQuery>,
    Query(f): Query<Filter>,
) -> Result<Html<String>> {
    let a2 = Arc::clone(&app);
    // The page reads its own parameters and the shared filter's period, so it keys the memo
    // by them: the chart and the commit subjects cost tens of ms on every visit otherwise.
    let period = f.period_value();
    let key = memo_query(&q, period);
    cached_keyed(&app, "l", key, move |a, r, c| {
        let head = a.repo.resolve(&crate::fmt::selected()).unwrap_or_default();
        let filter = path_filter(&a2, &head);
        let sk = a2.chain(&head, &**r, &filter);
        let dflt = a2.defaults().period.unwrap_or(Period::All);
        let mut view = View::parse(&q, period, sk.len(), dflt);
        if view.wants_stretch() && sk.len() > 0 {
            let scope = Scope::from_params(
                &ScopeParams::default(),
                a.defaults().into_options(),
                view.period,
            )
            .unwrap_or_else(|_| unreachable!("the default params are valid"));
            let ins = insights(a, r, scope.options().clone());
            let anchor = counted(&ins, &scope, r).context().anchor_day();
            let days = view.period.days().unwrap_or(1);
            let first = sk.first_from_day(&**r, anchor + 1 - days);
            view = view.with_stretch(first, sk.len());
        }
        let ch = sk.view(&**r, &filter, view);
        let at = shown(ch.len(), view);
        let ids: Vec<String> = at.iter().map(|i| ch.id(*i).to_owned()).collect();
        let mut by_id = crate::gitstate::subjects(a, &ids, "languages");
        let subj = Subjects(
            at.into_iter().zip(&ids).filter_map(|(i, id)| Some((i, by_id.remove(id)?))).collect(),
        );
        let body = body(&ch, view, &subj);
        let (lead, toolbar) = body.shown.as_ref().map_or_else(Default::default, |s| {
            (s.lead.clone(), format!("{}{}{}", s.period, s.axis, s.scale))
        });
        LanguagesPage {
            c,
            head: crate::pagehead::insights_head(
                "l",
                crate::pagehead::lead_tagline(&lead),
                toolbar,
            ),
            body,
        }
    })
    .await
}

/// The paths the page counts at `head`: the default options and the ignore rules there.
pub(crate) fn path_filter(app: &crate::app::App, head: &str) -> gitterrace_index::PathFilter {
    gitterrace_index::PathFilter::new(&gitterrace_index::Options::default(), app.ignore_set(head))
}

/// The memo key part of a query: every field the page reads, in a fixed order.
fn memo_query(q: &LocQuery, period: Option<Period>) -> String {
    format!(
        "x={}&period={}&scale={}&from={:?}&to={:?}",
        q.x.as_deref().unwrap_or_default(),
        period.map(Period::as_str).unwrap_or_default(),
        q.scale.as_deref().unwrap_or_default(),
        q.from,
        q.to
    )
}

#[cfg(test)]
mod tests;
