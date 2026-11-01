//! Commit types page: a pivot over type, scope, directory and author. Every row is a link that
//! adds a filter, and every number respects all active filters.

#![expect(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "geometry and colour math reads best with x, y, w, h; one linear sequence, splitting it hides the flow"
)]

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::{CommitRecord, Label, LabelCount, Period};
use serde::Deserialize;

use crate::{
    charts,
    error::Result,
    filter::{Dim, Filter, FilterBar, NO_SCOPE},
    fmt::{self, href},
    insights::{InsightsQuery, Kpi, OptLink, insights, period_of, scoped},
    routes::Shared,
    views::{self, Chrome, CommitRow, Row, Tabs},
};

const BASE: &str = "/stats/types";
const PAGE: usize = 20;
const TOP_KINDS: usize = 10;
const TOP_SCOPES: usize = 8;
const LIST: usize = 12;

#[derive(Deserialize, Default)]
pub(crate) struct TypesQuery {
    #[serde(flatten)]
    q: InsightsQuery,
    /// A string: inside a flattened query every value arrives as one.
    page: Option<String>,
}

struct Group {
    title: &'static str,
    hint: &'static str,
    lines: bool,
    rows: Vec<Row>,
}

struct Cell {
    n: usize,
    level: u8,
    href: String,
    scope: String,
}

struct MatrixRow {
    label: String,
    href: String,
    cells: Vec<Cell>,
}

struct MatrixCol {
    label: String,
    href: String,
}

struct Matrix {
    cols: Vec<MatrixCol>,
    rows: Vec<MatrixRow>,
}

#[derive(Template)]
#[template(path = "stats_types.html")]
struct TypesPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    kinds: Vec<Row>,
    matrix: Matrix,
    over_title: String,
    over_hint: String,
    over_time: String,
    groups: Vec<Group>,
    commits_total: usize,
    commits: Vec<CommitRow>,
    newer: Option<String>,
    older: Option<String>,
}

fn label_rows(v: &[LabelCount], link: impl Fn(&str) -> String) -> Vec<Row> {
    let max = v.first().map_or(0, |l| l.commits as u64);
    v.iter()
        .take(LIST)
        .map(|l| Row {
            pct: charts::pct(l.commits as u64, max),
            label: l.label.clone(),
            href: link(&l.label),
            commits: fmt::thousands(l.commits),
            additions: String::new(),
            deletions: String::new(),
        })
        .collect()
}

/// Shade level 1..=4 of `n` against the matrix maximum, 0 for an empty cell.
fn level(n: usize, max: usize) -> u8 {
    if n == 0 || max == 0 {
        return 0;
    }
    let l = (n * 4).div_ceil(max).clamp(1, 4);
    u8::try_from(l).unwrap_or(4)
}

fn matrix(
    recs: &[&CommitRecord],
    kinds: &[LabelCount],
    scopes: &[LabelCount],
    f: &Filter,
) -> Matrix {
    let kinds: Vec<String> = kinds
        .iter()
        .take(TOP_KINDS)
        .map(|l| l.label.clone())
        .collect();
    let scopes: Vec<String> = scopes
        .iter()
        .filter(|l| l.label != NO_SCOPE)
        .take(TOP_SCOPES)
        .map(|l| l.label.clone())
        .collect();
    let mut grid: HashMap<(&str, &str), usize> = HashMap::new();
    for r in recs {
        *grid
            .entry((Label::Kind.of(r), Label::Scope.of(r)))
            .or_default() += 1;
    }
    let max = scopes
        .iter()
        .flat_map(|s| kinds.iter().map(move |k| (k, s)))
        .map(|(k, s)| grid.get(&(k.as_str(), s.as_str())).copied().unwrap_or(0))
        .max()
        .unwrap_or(0);
    Matrix {
        cols: scopes
            .iter()
            .map(|s| MatrixCol {
                label: s.clone(),
                href: f.link(BASE, Dim::Scope, s),
            })
            .collect(),
        rows: kinds
            .iter()
            .map(|k| MatrixRow {
                label: k.clone(),
                href: f.link(BASE, Dim::Kind, k),
                cells: scopes
                    .iter()
                    .map(|s| {
                        let n = grid.get(&(k.as_str(), s.as_str())).copied().unwrap_or(0);
                        Cell {
                            n,
                            level: level(n, max),
                            href: href(&format!(
                                "{BASE}{}",
                                f.with(Dim::Kind, Some(k))
                                    .with(Dim::Scope, Some(s))
                                    .query(&[])
                            )),
                            scope: s.clone(),
                        }
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn churn_rows(v: Vec<(String, String, usize, u64, u64)>) -> Vec<Row> {
    let max = v.first().map_or(0, |r| r.2 as u64);
    v.into_iter()
        .take(LIST)
        .map(|(label, href, commits, add, del)| Row {
            pct: charts::pct(commits as u64, max),
            label,
            href,
            commits: fmt::thousands(commits),
            additions: fmt::thousands(add),
            deletions: fmt::thousands(del),
        })
        .collect()
}

fn page_link(f: &Filter, n: usize) -> String {
    href(&format!("{BASE}{}", f.query(&[("page", &n.to_string())])))
}

fn build(a: &crate::app::App, all: &[CommitRecord], c: Chrome, tq: &TypesQuery) -> TypesPage {
    let q = &tq.q;
    let f = &q.filter;
    let d = a.defaults();
    let scoped = scoped(f, all);
    let ins = insights(a, &scoped, q.options(&d));
    let period = period_of(q, &d, Period::All);
    let recs: Vec<&CommitRecord> = ins.counted_records(period).collect();
    let n = recs.len();
    let sum = ins.period_summary(period).current;
    let people = ins.contributors(period).contributors;
    let size = if n == 0 {
        0
    } else {
        (sum.additions + sum.deletions) / n as u64
    };
    let kpis = vec![
        Kpi("Commits", fmt::thousands(n)),
        Kpi("Authors", fmt::thousands(sum.authors)),
        Kpi("Lines added", format!("+{}", fmt::thousands(sum.additions))),
        Kpi(
            "Lines removed",
            format!("-{}", fmt::thousands(sum.deletions)),
        ),
        Kpi("Average commit", fmt::count(size, "line", "lines")),
    ];

    let kind_counts = ins.label_breakdown(Label::Kind, period);
    let scope_counts = ins.label_breakdown(Label::Scope, period);
    let (kinds, mx) = if f.kind().is_some() {
        (
            Vec::new(),
            Matrix {
                cols: Vec::new(),
                rows: Vec::new(),
            },
        )
    } else {
        (
            label_rows(&kind_counts, |l| f.link(BASE, Dim::Kind, l)),
            matrix(&recs, &kind_counts, &scope_counts, f),
        )
    };

    let by_scope = f.kind().is_some();
    let over = ins.label_over_time(if by_scope { Label::Scope } else { Label::Kind }, period);
    let unit = ins.bucket_unit(period).noun();
    let (over_title, over_hint) = if by_scope {
        (
            format!("Scopes over time, per {unit}"),
            "Commits of the chosen type, stacked by scope.".to_owned(),
        )
    } else {
        (
            format!("Types over time, per {unit}"),
            "Commits stacked by type; the biggest types sit at the bottom.".to_owned(),
        )
    };

    let mut groups = Vec::new();
    if f.scope().is_none() {
        groups.push(Group {
            title: "Scopes",
            hint: "The part of the code a commit says it touches, from type(scope). Click one to narrow to it; none means no scope given.",
            lines: false,
            rows: label_rows(&scope_counts, |l| f.link(BASE, Dim::Scope, l)),
        });
    }
    let dirs = ins
        .dir_churn(period)
        .into_iter()
        .map(|d| {
            let h = if d.path == "." {
                String::new()
            } else {
                f.link(BASE, Dim::Path, &d.path)
            };
            (
                if d.path == "." {
                    "(repository root)".to_owned()
                } else {
                    d.path
                },
                h,
                d.commits,
                d.additions,
                d.deletions,
            )
        })
        .collect();
    groups.push(Group {
        title: "Directories touched",
        hint: "Where these commits changed files, by directory, with lines added and removed. Click one to restrict to that path.",
        lines: true,
        rows: churn_rows(dirs),
    });
    let files = ins
        .file_churn(period)
        .into_iter()
        .map(|x| {
            (
                x.path.clone(),
                href(&format!(
                    "/blob/{}/{}",
                    fmt::selected_seg(),
                    fmt::enc_path(&x.path)
                )),
                x.commits,
                x.additions,
                x.deletions,
            )
        })
        .collect();
    groups.push(Group {
        title: "Files touched most",
        hint: "Churn: how many of these commits changed each file. Links open the file.",
        lines: true,
        rows: churn_rows(files),
    });
    if f.author().is_none() {
        let people = people
            .iter()
            .map(|p| {
                let email = p.emails.first().map_or(p.name.as_str(), String::as_str);
                (
                    p.name.clone(),
                    f.link(BASE, Dim::Author, email),
                    p.commits,
                    p.additions,
                    p.deletions,
                )
            })
            .collect();
        groups.push(Group {
            title: "Who",
            hint: "Authors of these commits, by commit count. Click a name to add them as a filter.",
            lines: true,
            rows: churn_rows(people),
        });
    }

    let mut list = recs;
    list.sort_by_key(|r| std::cmp::Reverse(r.time));
    let pg: usize = tq.page.as_deref().and_then(|p| p.parse().ok()).unwrap_or(0);
    let now = fmt::now();
    let shown: Vec<&CommitRecord> = list
        .iter()
        .skip(pg.saturating_mul(PAGE))
        .take(PAGE)
        .copied()
        .collect();
    // Only this page's subjects: scanning the whole history for 20 lines cost seconds on a
    // large repository.
    let ids: Vec<String> = shown.iter().map(|r| r.id.clone()).collect();
    let subjects: HashMap<String, String> = a
        .repo
        .summaries(&ids)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.id, s.subject))
        .collect();
    let commits = shown
        .iter()
        .map(|r| CommitRow {
            sha: fmt::sha_link(&r.id),
            initial: views::initial(&r.author),
            hue: views::hue(&r.author),
            href: href(&format!("/commit/{}", r.id)),
            subject: subjects.get(&r.id).cloned().unwrap_or_default(),
            author: r.author.clone(),
            author_href: f.link(BASE, Dim::Author, &r.email),
            when: fmt::ago(r.time, now),
            date: fmt::date(r.time, r.offset),
        })
        .collect();

    TypesPage {
        c,
        tabs: views::stats_tabs("t"),
        fb: f.bar(BASE, all),
        opts: q.row(&d, BASE),
        kpis,
        kinds,
        matrix: mx,
        over_title,
        over_hint,
        over_time: charts::stacked(&over),
        groups,
        commits_total: n,
        commits,
        newer: (pg > 0).then(|| page_link(f, pg - 1)),
        older: ((pg + 1) * PAGE < n).then(|| page_link(f, pg + 1)),
    }
}

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<TypesQuery>,
) -> Result<Html<String>> {
    let q = Arc::new(q);
    crate::insights::cached(&app, "t", raw, move |a, r, c| build(a, r, c, &q)).await
}

#[cfg(test)]
mod tests {
    use super::level;

    #[test]
    fn shade_levels_cover_the_ramp() {
        assert_eq!(level(5, 0), 0, "zero max stays on the lowest shade");
        assert_eq!(level(10, 10), 4, "the max count takes the top shade");
    }
}
