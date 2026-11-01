//! Commit types page: a pivot over type, scope, directory and author. Every row is a link that
//! adds a filter, and every number respects all active filters.

#![allow(clippy::many_single_char_names, clippy::too_many_lines)]

use std::collections::HashMap;
use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::{CommitRecord, LabelBucket, LabelCount, TimeAxis};
use serde::Deserialize;

use crate::{
    charts,
    error::Result,
    filter::{Dim, Filter, FilterBar, NO_SCOPE, OTHER},
    fmt::{self, href},
    routes::Shared,
    views::{self, Chrome, CommitRow, Nav, Row},
};

const BASE: &str = "/stats/types";
const PAGE: usize = 20;
const TOP_KINDS: usize = 10;
const TOP_SCOPES: usize = 8;
const LIST: usize = 12;

#[derive(Deserialize, Default)]
pub(crate) struct TypesQuery {
    #[serde(flatten)]
    filter: Filter,
    page: Option<usize>,
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
    tabs: Vec<Nav>,
    fb: FilterBar,
    kpis: Vec<(&'static str, String)>,
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

fn line_totals(r: &CommitRecord) -> (u64, u64) {
    r.files.iter().fold((0, 0), |(a, d), f| {
        (a + u64::from(f.additions), d + u64::from(f.deletions))
    })
}

fn kind_of(r: &CommitRecord) -> &str {
    r.kind.as_deref().unwrap_or(OTHER)
}

fn scope_of(r: &CommitRecord) -> &str {
    r.scope.as_deref().unwrap_or(NO_SCOPE)
}

fn ranked(map: HashMap<&str, usize>) -> Vec<LabelCount> {
    let mut v: Vec<LabelCount> = map
        .into_iter()
        .map(|(label, commits)| LabelCount {
            label: label.to_owned(),
            commits,
        })
        .collect();
    v.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.label.cmp(&b.label))
    });
    v
}

fn count<'a>(
    recs: &'a [CommitRecord],
    key: impl Fn(&'a CommitRecord) -> &'a str,
) -> Vec<LabelCount> {
    let mut m: HashMap<&str, usize> = HashMap::new();
    for r in recs {
        *m.entry(key(r)).or_default() += 1;
    }
    ranked(m)
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

fn matrix(recs: &[CommitRecord], f: &Filter) -> Matrix {
    let kinds: Vec<String> = count(recs, kind_of)
        .into_iter()
        .take(TOP_KINDS)
        .map(|l| l.label)
        .collect();
    let scopes: Vec<String> = count(recs, scope_of)
        .into_iter()
        .filter(|l| l.label != NO_SCOPE)
        .take(TOP_SCOPES)
        .map(|l| l.label)
        .collect();
    let mut grid: HashMap<(&str, &str), usize> = HashMap::new();
    for r in recs {
        *grid.entry((kind_of(r), scope_of(r))).or_default() += 1;
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

/// Commits per time bucket, split by type (or by scope once a type is chosen).
fn over_time(recs: &[CommitRecord], by_scope: bool, axis: &TimeAxis) -> String {
    let mut maps: Vec<HashMap<&str, usize>> = vec![HashMap::new(); axis.len()];
    for r in recs {
        if let Some(i) = axis.slot(r) {
            let label = if by_scope { scope_of(r) } else { kind_of(r) };
            *maps[i].entry(label).or_default() += 1;
        }
    }
    let buckets: Vec<LabelBucket> = axis
        .names()
        .zip(maps)
        .map(|((key, label), m)| LabelBucket {
            key,
            label,
            counts: ranked(m),
        })
        .collect();
    charts::stacked(&buckets)
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

fn build(a: &crate::app::App, all: &[CommitRecord], c: Chrome, q: &TypesQuery) -> TypesPage {
    let f = &q.filter;
    let recs = f.apply(all);
    let recs: &[CommitRecord] = &recs;
    let (adds, dels) = recs.iter().fold((0u64, 0u64), |(x, y), r| {
        let (p, d) = line_totals(r);
        (x + p, y + d)
    });
    let n = recs.len();
    let authors = gitterrace_index::authors(recs);
    let size = if n == 0 { 0 } else { (adds + dels) / n as u64 };
    let kpis = vec![
        ("Commits", fmt::thousands(n)),
        ("Authors", fmt::thousands(authors.len())),
        ("Lines added", format!("+{}", fmt::thousands(adds))),
        ("Lines removed", format!("-{}", fmt::thousands(dels))),
        ("Average commit", format!("{} lines", fmt::thousands(size))),
    ];

    let kinds = if f.kind().is_some() {
        Vec::new()
    } else {
        label_rows(&count(recs, kind_of), |l| f.link(BASE, Dim::Kind, l))
    };
    let mx = if f.kind().is_some() {
        Matrix {
            cols: Vec::new(),
            rows: Vec::new(),
        }
    } else {
        matrix(recs, f)
    };

    let axis = TimeAxis::covering(f.period_value(), all, recs);
    let by_scope = f.kind().is_some();
    let unit = axis.unit().noun();
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
        let scopes: Vec<LabelCount> = count(recs, scope_of);
        groups.push(Group {
            title: "Scopes",
            hint: "The part of the code a commit says it touches, from type(scope). Click one to narrow to it; none means no scope given.",
            lines: false,
            rows: label_rows(&scopes, |l| f.link(BASE, Dim::Scope, l)),
        });
    }
    let dirs = gitterrace_index::dir_churn(recs)
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
    let files = gitterrace_index::file_churn(recs)
        .into_iter()
        .map(|x| {
            (
                x.path.clone(),
                href(&format!("/blob/HEAD/{}", fmt::enc(&x.path, true))),
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
        let people = authors
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    f.link(BASE, Dim::Author, &p.email),
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

    let head = a.repo.resolve("HEAD").unwrap_or_default();
    let subjects = a.subjects(&head);
    let mut list: Vec<&CommitRecord> = recs.iter().collect();
    list.sort_by_key(|r| std::cmp::Reverse(r.time));
    let pg = q.page.unwrap_or(0);
    let now = fmt::now();
    let commits = list
        .iter()
        .skip(pg.saturating_mul(PAGE))
        .take(PAGE)
        .map(|r| CommitRow {
            short: fmt::short(&r.id),
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
        kpis,
        kinds,
        matrix: mx,
        over_title,
        over_hint,
        over_time: over_time(recs, by_scope, &axis),
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
        assert_eq!(level(0, 10), 0);
        assert_eq!(level(1, 10), 1);
        assert_eq!(level(10, 10), 4);
        assert_eq!(level(5, 0), 0);
    }
}
