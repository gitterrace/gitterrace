//! Languages page: lines of code per language at every first-parent commit, a brush that
//! selects a commit range, and the range card the server renders from `?from&to`.

#![expect(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    clippy::needless_range_loop,
    reason = "geometry and colour math reads best with x, y, w, h; one linear sequence, splitting it hides the flow; the index is used for several parallel arrays"
)]

mod render;

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use gitterrace_index::Insights;
use gitterrace_index::feed::{PathId, Rank};
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

/// Lines per language after each commit of the first-parent chain, oldest first.
pub(crate) struct Chain {
    ids: Vec<String>,
    times: Vec<i64>,
    offsets: Vec<i32>,
    adds: Vec<u64>,
    dels: Vec<u64>,
    /// Top languages, largest first, then `Other` when folded.
    langs: Vec<String>,
    /// `lines[commit][language]`.
    lines: Vec<Vec<i64>>,
}

impl Chain {
    /// Walks the first-parent chain from the newest commit and streams only its file rows:
    /// the state is one running total per language, never the rows.
    pub(crate) fn from_source(ins: &Insights<'_>) -> Self {
        let src = ins.source();
        let mut chain: Vec<Rank> = Vec::new();
        let mut at = src
            .len()
            .checked_sub(1)
            .map(|i| Rank(u32::try_from(i).unwrap_or(u32::MAX)));
        while let Some(r) = at {
            chain.push(r);
            at = src.parent_ids(r).first().and_then(|p| src.find_commit(p));
        }
        chain.reverse();

        let mut names: Vec<String> = Vec::new();
        let mut slot: HashMap<String, usize> = HashMap::new();
        // Language slot per path id, `None` when the path filter leaves it out.
        let mut lang_of: HashMap<PathId, Option<usize>> = HashMap::new();
        let mut running: Vec<i64> = Vec::new();
        let mut rows: Vec<Vec<i64>> = Vec::with_capacity(chain.len());
        let (mut adds, mut dels) = (Vec::new(), Vec::new());
        for (_, files) in src.rows_for(chain.clone()) {
            let (mut a, mut d) = (0u64, 0u64);
            for f in &files {
                let s = *lang_of.entry(f.path).or_insert_with(|| {
                    let path = src.path(f.path).unwrap_or_default();
                    if ins.skips(&path) {
                        return None;
                    }
                    let l = gitterrace_index::language(&path);
                    Some(*slot.entry(l.clone()).or_insert_with(|| {
                        names.push(l);
                        running.push(0);
                        names.len() - 1
                    }))
                });
                let Some(s) = s else {
                    continue;
                };
                running[s] += i64::from(f.add) - i64::from(f.del);
                a += u64::from(f.add);
                d += u64::from(f.del);
            }
            adds.push(a);
            dels.push(d);
            rows.push(running.clone());
        }
        let last = rows.last().cloned().unwrap_or_default();
        let mut order: Vec<usize> = (0..names.len()).filter(|s| last[*s] > 0).collect();
        order.sort_by(|a, b| {
            last[*b]
                .cmp(&last[*a])
                .then_with(|| names[*a].cmp(&names[*b]))
        });
        let keep: Vec<usize> = if order.len() <= TOP + 1 {
            order
        } else {
            order[..TOP].to_vec()
        };
        let folded = names.len() > keep.len();
        let mut langs: Vec<String> = keep.iter().map(|s| names[*s].clone()).collect();
        if folded {
            langs.push("Other".to_owned());
        }
        let lines = rows
            .iter()
            .map(|row| {
                let mut v: Vec<i64> = keep
                    .iter()
                    .map(|s| row.get(*s).copied().unwrap_or(0))
                    .collect();
                if folded {
                    let kept: i64 = v.iter().sum();
                    let all: i64 = row.iter().sum();
                    v.push(all - kept);
                }
                v
            })
            .collect();
        Self {
            ids: chain
                .iter()
                .map(|r| src.commit_id(*r).map(|s| s.to_string()).unwrap_or_default())
                .collect(),
            times: chain
                .iter()
                .map(|r| src.commit(*r).map_or(0, |c| c.time))
                .collect(),
            offsets: chain
                .iter()
                .map(|r| src.commit(*r).map_or(0, |c| c.offset))
                .collect(),
            adds,
            dels,
            langs,
            lines,
        }
    }

    const fn len(&self) -> usize {
        self.ids.len()
    }

    /// Chain positions whose subject the page shows: the chart samples and the newest
    /// commits of the range list. Everything else is never read, so it is never fetched.
    fn shown(&self, view: View) -> Vec<usize> {
        let n = self.len();
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

    /// Lines per language before commit `i` (all zero before the first).
    fn before(&self, i: usize) -> Vec<i64> {
        if i == 0 {
            vec![0; self.langs.len()]
        } else {
            self.lines[i - 1].clone()
        }
    }
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
        let ch = a2.chain(&head, &**r);
        let view = View::parse(&q, ch.len());
        let at = ch.shown(view);
        let ids: Vec<String> = at.iter().map(|i| ch.ids[*i].clone()).collect();
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
