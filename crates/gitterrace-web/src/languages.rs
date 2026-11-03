//! Languages page: lines of code per language along the first-parent history, a brush that
//! selects a commit range, and the range card the server renders from `?from&to`.

mod chain;
mod render;

pub(crate) use chain::Skeleton;

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::{
    error::Result,
    fmt,
    insights::cached_keyed,
    languages::render::{body, sample},
    routes::Shared,
    views::{self, Chrome, Tabs},
};

const TOP: usize = 7;
const W: f64 = 960.0;
const H: f64 = 280.0;
const LEFT: f64 = 72.0;
const RIGHT: f64 = 8.0;
const TOP_PAD: f64 = 10.0;
const BOTTOM: f64 = 26.0;
/// Points kept in the inline JSON and the area paths.
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

/// View state carried by every link on the page.
#[derive(Clone, Copy)]
struct View {
    time: bool,
    share: bool,
    range: Option<(usize, usize)>,
}

impl View {
    fn parse(q: &LocQuery, n: usize) -> Self {
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
        }
    }

    fn href(self, f: impl FnOnce(&mut Self)) -> String {
        let mut v = self;
        f(&mut v);
        let mut q: Vec<String> = Vec::new();
        if v.time {
            q.push("x=time".to_owned());
        }
        if v.share {
            q.push("scale=share".to_owned());
        }
        if let Some((a, b)) = v.range {
            q.push(format!("from={a}"));
            q.push(format!("to={b}"));
        }
        if q.is_empty() {
            fmt::href("/stats/languages")
        } else {
            fmt::href(&format!("/stats/languages?{}", q.join("&")))
        }
    }
}

#[derive(Template)]
#[template(path = "languages.html")]
struct LanguagesPage {
    c: Chrome,
    tabs: Tabs,
    body: String,
}

pub(crate) async fn page(State(app): Shared, Query(q): Query<LocQuery>) -> Result<Html<String>> {
    let a2 = Arc::clone(&app);
    // The page reads its own parameters, none of the shared filter ones, so it keys the memo
    // by them: the chart and the commit subjects cost tens of ms on every visit otherwise.
    let key = memo_query(&q);
    cached_keyed(&app, "l", key, move |a, r, c| {
        let head = a.repo.resolve(&crate::fmt::selected()).unwrap_or_default();
        let filter = path_filter(&a2, &head);
        let sk = a2.chain(&head, &**r, &filter);
        let view = View::parse(&q, sk.len());
        let ch = sk.view(&**r, &filter, view);
        let at = shown(ch.len(), view);
        let ids: Vec<String> = at.iter().map(|i| ch.id(*i).to_owned()).collect();
        let subj = match a.repo.summaries(&ids) {
            Ok(sums) => Subjects(
                at.into_iter()
                    .zip(sums.into_iter().map(|s| s.subject))
                    .collect(),
            ),
            Err(e) => {
                tracing::warn!("subjects unavailable: {e}");
                Subjects(HashMap::new())
            }
        };
        LanguagesPage {
            c,
            tabs: views::stats_tabs("l"),
            body: body(&ch, view, &subj),
        }
    })
    .await
}

/// The paths the page counts at `head`: the default options and the ignore rules there.
pub(crate) fn path_filter(app: &crate::app::App, head: &str) -> gitterrace_index::PathFilter {
    gitterrace_index::PathFilter::new(&gitterrace_index::Options::default(), app.ignore_set(head))
}

/// The memo key part of a query: every field the page reads, in a fixed order.
fn memo_query(q: &LocQuery) -> String {
    format!(
        "x={}&scale={}&from={:?}&to={:?}",
        q.x.as_deref().unwrap_or_default(),
        q.scale.as_deref().unwrap_or_default(),
        q.from,
        q.to
    )
}

#[cfg(test)]
mod tests;
