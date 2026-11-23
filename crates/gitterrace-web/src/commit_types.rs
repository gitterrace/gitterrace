//! The commit types section of the activity page: a pivot over type, scope, directory and
//! author. Every row is a link that adds a filter, and every number respects all active filters.

#![expect(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "geometry and colour math reads best with x, y, w, h; one linear sequence, splitting \
        it hides the flow"
)]

use std::collections::HashMap;

use askama::Template;
use gitterrace_index::feed::{CommitRow as SrcRow, InsightsSource, LabelId};
use gitterrace_index::{ActivityFold, Counted, LabelCount, Selection};

use crate::{
    charts,
    filter::{Dim, Filter, NO_SCOPE},
    fmt::{self, href},
    insights::{InsightsQuery, Kpi},
    views::{self, CommitRow, Row},
};

/// The page the section lives on: every link it writes narrows that page.
const BASE: &str = "/stats";
const PAGE: usize = 20;
const TOP_KINDS: usize = 10;
const TOP_SCOPES: usize = 8;
const LIST: usize = 12;

struct Group {
    title: &'static str,
    hint: &'static str,
    lines: bool,
    rows: Vec<Row>,
}

pub(crate) struct Cell {
    pub(crate) n: usize,
    pub(crate) level: u8,
    pub(crate) href: String,
    pub(crate) tip: String,
}

pub(crate) struct MatrixRow {
    pub(crate) label: String,
    pub(crate) href: String,
    pub(crate) cells: Vec<Cell>,
}

pub(crate) struct MatrixCol {
    pub(crate) label: String,
    pub(crate) href: String,
}

pub(crate) struct Matrix {
    pub(crate) cols: Vec<MatrixCol>,
    pub(crate) rows: Vec<MatrixRow>,
}

/// The type by scope grid as a chart figure: a table of links whose cells are shaded by count,
/// named, summarised, and its counts with row and column totals in the data table, so it reads
/// like every other chart.
#[derive(Template)]
#[template(path = "matrix.html")]
pub(crate) struct MatrixFigure {
    pub(crate) m: Matrix,
    pub(crate) fig: charts::view::Fig,
}

impl askama::filters::HtmlSafe for MatrixFigure {}

fn matrix_figure(m: Matrix) -> Option<MatrixFigure> {
    if m.cols.is_empty() || m.rows.is_empty() {
        return None;
    }
    let width = m.cols.len();
    let col_sums: Vec<u64> =
        (0..width).map(|c| m.rows.iter().map(|r| r.cells[c].n as u64).sum()).collect();
    let all: u64 = col_sums.iter().sum();
    let row = |label: &str, vals: &[u64]| {
        let mut cells: Vec<String> = vals.iter().map(|v| fmt::thousands(*v)).collect();
        cells.push(fmt::thousands(vals.iter().sum::<u64>()));
        charts::view::Row { label: label.to_owned(), cells }
    };
    let mut head = vec!["Type".to_owned()];
    head.extend(m.cols.iter().map(|c| c.label.clone()));
    head.push("Commits".to_owned());
    let table = charts::view::Table {
        head,
        rows: m
            .rows
            .iter()
            .map(|r| {
                let vals: Vec<u64> = r.cells.iter().map(|c| c.n as u64).collect();
                row(&r.label, &vals)
            })
            .collect(),
        total: Some(row("Total", &col_sums)),
    };
    let mut top: Option<(&str, &str, usize)> = None;
    for r in &m.rows {
        for (c, cell) in m.cols.iter().zip(&r.cells) {
            if cell.n > top.map_or(0, |t| t.2) {
                top = Some((&r.label, &c.label, cell.n));
            }
        }
    }
    let summary = top.map_or_else(
        || "No commits with a type and a scope.".to_owned(),
        |(k, sc, n)| {
            format!(
                "{} by type and scope, the most {k} in {sc} with {}.",
                fmt::count(all, "commit", "commits"),
                fmt::thousands(n)
            )
        },
    );
    let fig = charts::fig("Type by scope", summary, table);
    Some(MatrixFigure { m, fig })
}

#[derive(Template)]
#[template(path = "commit_types_section.html")]
struct TypesSection {
    kpis: Vec<Kpi>,
    kinds: Vec<Row>,
    matrix_fig: Option<MatrixFigure>,
    over_title: String,
    over_hint: String,
    over_time: crate::charts::view::Stacked,
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
pub(crate) fn level(n: usize, max: usize) -> u8 {
    if n == 0 || max == 0 {
        return 0;
    }
    let l = (n * 4).div_ceil(max).clamp(1, 4);
    u8::try_from(l).unwrap_or(4)
}

fn matrix(
    src: &dyn InsightsSource,
    recs: &[SrcRow],
    kinds: &[LabelCount],
    scopes: &[LabelCount],
    f: &Filter,
) -> Matrix {
    let kinds: Vec<String> = kinds.iter().take(TOP_KINDS).map(|l| l.label.clone()).collect();
    let scopes: Vec<String> = scopes
        .iter()
        .filter(|l| l.label != NO_SCOPE)
        .take(TOP_SCOPES)
        .map(|l| l.label.clone())
        .collect();
    let mut ids: HashMap<(Option<LabelId>, Option<LabelId>), usize> = HashMap::new();
    for r in recs {
        *ids.entry((r.kind, r.scope)).or_default() += 1;
    }
    let name = |id: Option<LabelId>, missing: &str| -> String {
        id.and_then(|i| src.label(i)).map_or_else(|| missing.to_owned(), |s| s.to_string())
    };
    let mut named: HashMap<(String, String), usize> = HashMap::new();
    for ((k, sc), n) in ids {
        *named.entry((name(k, Selection::OTHER), name(sc, NO_SCOPE))).or_default() += n;
    }
    let grid: HashMap<(&str, &str), usize> =
        named.iter().map(|((k, sc), n)| ((k.as_str(), sc.as_str()), *n)).collect();
    let count = |k: &str, s: &str| grid.get(&(k, s)).copied().unwrap_or(0);
    let counts = || scopes.iter().flat_map(|s| kinds.iter().map(move |k| count(k, s)));
    let (all, max) = (counts().sum::<usize>(), counts().max().unwrap_or(0));
    Matrix {
        cols: scopes
            .iter()
            .map(|s| MatrixCol { label: s.clone(), href: f.link(BASE, Dim::Scope, s) })
            .collect(),
        rows: kinds
            .iter()
            .map(|k| MatrixRow {
                label: k.clone(),
                href: f.link(BASE, Dim::Kind, k),
                cells: scopes
                    .iter()
                    .map(|s| {
                        let n = count(k, s);
                        Cell {
                            n,
                            level: level(n, max),
                            href: href(&format!(
                                "{BASE}{}",
                                f.with(Dim::Kind, Some(k)).with(Dim::Scope, Some(s)).query(&[])
                            )),
                            tip: crate::charttip::part(
                                &format!("{k} / {s}"),
                                n as u64,
                                all as u64,
                                crate::charttip::COMMITS,
                            ),
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
    href(&format!("{BASE}{}#commits", f.query(&[("page", &n.to_string())])))
}

/// The section's markup for the commits `q` selects, at commit-list page `page`.
pub(crate) fn section(
    a: &crate::app::App,
    q: &InsightsQuery,
    page: Option<&str>,
    set: &Counted<'_>,
    fold: &ActivityFold,
) -> String {
    let f = &q.filter;
    let src = set.insights().source();
    let recs: Vec<SrcRow> = fold.rows.clone();
    let sum = &fold.summary.current;
    let n = sum.commits;
    let people = &fold.people.contributors;
    let size = if n == 0 { 0 } else { (sum.additions + sum.deletions) / n as u64 };
    let kpis = vec![
        Kpi("Commits without merges", fmt::thousands(n)),
        Kpi("Authors", fmt::thousands(sum.authors)),
        Kpi("Lines added", format!("+{}", fmt::thousands(sum.additions))),
        Kpi("Lines removed", format!("-{}", fmt::thousands(sum.deletions))),
        Kpi("Average commit", fmt::count(size, "line", "lines")),
    ];

    let (kind_counts, scope_counts) = (&fold.kinds.0, &fold.scopes.0);
    let (kinds, mx) = if f.kind().is_some() {
        (Vec::new(), Matrix { cols: Vec::new(), rows: Vec::new() })
    } else {
        (
            label_rows(kind_counts, |l| f.link(BASE, Dim::Kind, l)),
            matrix(src, &recs, kind_counts, scope_counts, f),
        )
    };

    let by_scope = f.kind().is_some();
    let over = if by_scope { &fold.scopes.1 } else { &fold.kinds.1 };
    let unit = set.axis().unit().noun();
    let (over_title, over_hint) = if by_scope {
        (
            format!("Scopes over time, per {unit}"),
            "Commits of the chosen type, stacked by scope.".to_owned(),
        )
    } else {
        (
            format!("Types over time, per {unit}"),
            "Commits stacked by type. The biggest types sit at the bottom.".to_owned(),
        )
    };

    let mut groups = Vec::new();
    if f.scope().is_none() {
        groups.push(Group {
            title: "Scopes",
            hint: "The part of the code a commit says it touches, from type(scope). Click one to \
                narrow to it. None means no scope given.",
            lines: false,
            rows: label_rows(scope_counts, |l| f.link(BASE, Dim::Scope, l)),
        });
    }
    let dirs = fold
        .dirs
        .iter()
        // Only the first LIST rows render: links for the rest cost most of a cold build.
        .take(LIST)
        .map(|d| {
            let h = if d.path == "." { String::new() } else { f.link(BASE, Dim::Path, &d.path) };
            (
                if d.path == "." { "(repository root)".to_owned() } else { d.path.clone() },
                h,
                d.commits,
                d.additions,
                d.deletions,
            )
        })
        .collect();
    groups.push(Group {
        title: "Directories touched",
        hint: "Where these commits changed files, by directory, with lines added and removed. \
            Click one to restrict to that path.",
        lines: true,
        rows: churn_rows(dirs),
    });
    let files = fold
        .files
        .iter()
        .take(LIST)
        .map(|x| {
            (
                x.path.clone(),
                href(&format!("/blob/{}/{}", fmt::selected_seg(), fmt::enc_path(&x.path))),
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
            .take(LIST)
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
            hint: "Authors of these commits, by commit count. Click a name to add them as a \
                filter.",
            lines: true,
            rows: churn_rows(people),
        });
    }

    let mut list = recs;
    // Equal times break by rank, never by storage order: the canonical order decides.
    list.sort_by_key(|r| (std::cmp::Reverse(r.time), std::cmp::Reverse(r.rank)));
    let pg: usize = page.and_then(|p| p.parse().ok()).unwrap_or(0);
    let now = fmt::now();
    // (id, author, email, time, offset) of this page's commits.
    let shown: Vec<(String, String, String, i64, i32)> = list
        .iter()
        .skip(pg.saturating_mul(PAGE))
        .take(PAGE)
        .map(|r| {
            let (author, email) =
                src.ident(r.ident).map(|(n, e)| (n.to_string(), e.to_string())).unwrap_or_default();
            let id = src.commit_id(r.rank).map(|s| s.to_string()).unwrap_or_default();
            (id, author, email, r.time, r.offset)
        })
        .collect();
    // Only this page's subjects: scanning the whole history for 20 lines cost seconds on a
    // large repository.
    let ids: Vec<String> = shown.iter().map(|r| r.0.clone()).collect();
    let subjects = crate::gitstate::subjects(a, &ids, "commit types");
    let commits = shown
        .iter()
        .map(|(id, author, email, time, offset)| CommitRow {
            sha: fmt::sha_link(id),
            initial: views::initials(author),
            hue: views::hue(author),
            href: href(&format!("/commit/{id}")),
            subject: subjects.get(id).cloned().unwrap_or_default(),
            author: author.clone(),
            author_href: f.link(BASE, Dim::Author, email),
            when: fmt::ago_at(*time, *offset, now),
            ..CommitRow::default()
        })
        .collect();

    let over_time = charts::stacked(&over_title, over);
    TypesSection {
        kpis,
        kinds,
        matrix_fig: matrix_figure(mx),
        over_title,
        over_hint,
        over_time,
        groups,
        commits_total: n,
        commits,
        newer: (pg > 0).then(|| page_link(f, pg - 1)),
        older: ((pg + 1) * PAGE < n).then(|| page_link(f, pg + 1)),
    }
    .render()
    .unwrap_or_default()
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
