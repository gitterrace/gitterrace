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
use gitterrace_index::{Label, LabelCount, Period, Selection};

use crate::{
    charts,
    filter::{Dim, Filter, NO_SCOPE},
    fmt::{self, href},
    insights::{InsightsQuery, Kpi, Src, counted, insights, page_scope, period_of},
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
#[template(path = "commit_types_section.html")]
struct TypesSection {
    kpis: Vec<Kpi>,
    kinds: Vec<Row>,
    matrix: Matrix,
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
fn level(n: usize, max: usize) -> u8 {
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
    let mut ids: HashMap<(Option<LabelId>, Option<LabelId>), usize> = HashMap::new();
    for r in recs {
        *ids.entry((r.kind, r.scope)).or_default() += 1;
    }
    let name = |id: Option<LabelId>, missing: &str| -> String {
        id.and_then(|i| src.label(i))
            .map_or_else(|| missing.to_owned(), |s| s.to_string())
    };
    let mut named: HashMap<(String, String), usize> = HashMap::new();
    for ((k, sc), n) in ids {
        *named
            .entry((name(k, Selection::OTHER), name(sc, NO_SCOPE)))
            .or_default() += n;
    }
    let grid: HashMap<(&str, &str), usize> = named
        .iter()
        .map(|((k, sc), n)| ((k.as_str(), sc.as_str()), *n))
        .collect();
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
    href(&format!(
        "{BASE}{}#commits",
        f.query(&[("page", &n.to_string())])
    ))
}

/// The section's markup for the commits `q` selects, at commit-list page `page`.
pub(crate) fn section(
    a: &crate::app::App,
    all: &Src,
    q: &InsightsQuery,
    page: Option<&str>,
) -> String {
    let f = &q.filter;
    let d = a.defaults();
    let period = period_of(q, &d, Period::All);
    let scope = page_scope(q, &d, period);
    let ins = insights(a, all, scope.options().clone());
    let set = counted(&ins, &scope, all);
    let src = ins.source();
    let recs: Vec<SrcRow> = set.stream().collect();
    let n = recs.len();
    let sum = set.totals();
    let people = set
        .contributors(gitterrace_index::Metric::Commits)
        .contributors;
    let size = if n == 0 {
        0
    } else {
        (sum.additions + sum.deletions) / n as u64
    };
    let kpis = vec![
        Kpi("Commits without merges", fmt::thousands(n)),
        Kpi("Authors", fmt::thousands(sum.authors)),
        Kpi("Lines added", format!("+{}", fmt::thousands(sum.additions))),
        Kpi(
            "Lines removed",
            format!("-{}", fmt::thousands(sum.deletions)),
        ),
        Kpi("Average commit", fmt::count(size, "line", "lines")),
    ];

    let kind_counts = set.per_label(Label::Kind).0;
    let scope_counts = set.per_label(Label::Scope).0;
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
            matrix(src, &recs, &kind_counts, &scope_counts, f),
        )
    };

    let by_scope = f.kind().is_some();
    let over = set
        .per_label(if by_scope { Label::Scope } else { Label::Kind })
        .1;
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
            rows: label_rows(&scope_counts, |l| f.link(BASE, Dim::Scope, l)),
        });
    }
    let dirs = set
        .per_dir()
        .into_iter()
        // Only the first LIST rows render: links for the rest cost most of a cold build.
        .take(LIST)
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
        hint: "Where these commits changed files, by directory, with lines added and removed. \
            Click one to restrict to that path.",
        lines: true,
        rows: churn_rows(dirs),
    });
    let files = set
        .per_file()
        .into_iter()
        .take(LIST)
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
            let (author, email) = src
                .ident(r.ident)
                .map(|(n, e)| (n.to_string(), e.to_string()))
                .unwrap_or_default();
            let id = src
                .commit_id(r.rank)
                .map(|s| s.to_string())
                .unwrap_or_default();
            (id, author, email, r.time, r.offset)
        })
        .collect();
    // Only this page's subjects: scanning the whole history for 20 lines cost seconds on a
    // large repository.
    let ids: Vec<String> = shown.iter().map(|r| r.0.clone()).collect();
    let subjects: HashMap<String, String> = match a.repo.summaries(&ids) {
        Ok(s) => s.into_iter().map(|s| (s.id, s.subject)).collect(),
        Err(e) => {
            tracing::warn!("commit types: subjects unavailable: {e}");
            HashMap::new()
        }
    };
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

    TypesSection {
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
