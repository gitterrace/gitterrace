//! The coupled-files section of the hotspots page: the pairs of files that change in the same
//! commits, for the whole tree or for one focus file, with a sibling-only or cross-directory
//! switch and a matrix of the top-level directories.
//!
//! Every number is `Insights::coupling_report_in`, the one the API serves as
//! `/insights/coupling`. The section reads three URL keys of its own: `focus`, `cross_dir`
//! and `min_support`. They are part of the page memo key (`App::canonical_query`).

use askama::Template;
use gitterrace_index::{Counted, CouplingQuery, HeadFile, Insights};
use serde::Deserialize;

use crate::{
    charts,
    commit_types::{Cell, Matrix, MatrixCol, MatrixFigure, MatrixRow, level},
    empty::Empty,
    filter::Dim,
    fmt,
    insights::{Defaults, InsightsQuery},
    seg::{Choice, Seg},
};

/// Pairs listed. Not configurable: a page of the strongest pairs, the API takes `limit`.
const TOP_PAIRS: usize = 30;
/// Shared commits a pair needs unless `min_support` says otherwise.
/// Not configurable: the bar the patterns section has always used.
const MIN_SUPPORT: usize = 3;
/// Directories on each side of the matrix.
/// Not configurable: more columns than this stop fitting the page.
const MATRIX_DIRS: usize = 10;
const BASE: &str = "/stats/hotspots";

/// The keys of the section, read from the page's query next to the shared filter.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct CouplingParams {
    /// The file whose partners are listed.
    pub(crate) focus: Option<String>,
    /// `1` lists pairs across directories too.
    pub(crate) cross_dir: Option<String>,
    pub(crate) min_support: Option<String>,
}

impl CouplingParams {
    fn cross(&self) -> bool {
        self.cross_dir.as_deref() == Some("1")
    }

    fn support(&self) -> usize {
        self.min_support.as_deref().and_then(|v| v.parse().ok()).unwrap_or(MIN_SUPPORT).max(1)
    }

    fn focus(&self) -> Option<&str> {
        self.focus.as_deref().map(|f| f.trim_matches('/')).filter(|f| !f.is_empty())
    }

    /// The link to this section with the focus and the switch as given and the support kept.
    fn link(&self, q: &InsightsQuery, d: &Defaults, focus: Option<&str>, cross: bool) -> String {
        let support = self.support().to_string();
        let mut set: Vec<(&str, &str)> = Vec::new();
        if let Some(f) = focus {
            set.push(("focus", f));
        }
        if cross {
            set.push(("cross_dir", "1"));
        }
        if self.support() != MIN_SUPPORT {
            set.push(("min_support", &support));
        }
        format!("{}#coupled", q.link_with(d, BASE, &set))
    }
}

struct PairRow {
    path: String,
    file: Vec<String>,
    file_href: String,
    with: Vec<String>,
    with_href: String,
    focus_href: String,
    together: String,
    /// Of the file's changes, the share that came with the partner.
    of_file: String,
    of_with: String,
    last: String,
    cross: bool,
}

#[derive(Template)]
#[template(path = "coupling.html")]
struct CouplingSection {
    rows: Vec<PairRow>,
    kind: Seg,
    focus: Option<String>,
    all_href: String,
    threshold: String,
    matrix: Option<MatrixFigure>,
    none: Option<Empty>,
}

pub(crate) fn section(
    ins: &Insights<'_>,
    set: &Counted<'_>,
    files: &[HeadFile],
    query: &InsightsQuery,
    defaults: &Defaults,
    params: &CouplingParams,
) -> String {
    let focus = params.focus();
    let cross = params.cross();
    let rep = ins.coupling_report_in(
        files,
        set,
        &CouplingQuery {
            path: focus,
            min_support: params.support(),
            cross_dir: cross,
            limit: TOP_PAIRS,
        },
    );
    let rows: Vec<PairRow> = rep
        .pairs
        .iter()
        .map(|x| {
            // With a focus the focus is the file and its partner the other, whichever sorts first.
            let flip = focus == Some(x.b.as_str());
            let (file, with) = if flip { (&x.b, &x.a) } else { (&x.a, &x.b) };
            let (of_file, of_with) = if flip {
                (x.confidence_ba, x.confidence_ab)
            } else {
                (x.confidence_ab, x.confidence_ba)
            };
            PairRow {
                path: file.clone(),
                file: fmt::path_parts(file),
                file_href: fmt::file_link(file),
                with: fmt::path_parts(with),
                with_href: fmt::file_link(with),
                focus_href: params.link(query, defaults, Some(with), cross),
                together: fmt::thousands(x.together),
                of_file: format!("{:.0}%", of_file * 100.0),
                of_with: format!("{:.0}%", of_with * 100.0),
                last: fmt::day(x.last_together),
                cross: x.cross_dir,
            }
        })
        .collect();
    let kind = Seg::links(
        "Pairs",
        vec![
            Choice::new("Same directory", params.link(query, defaults, focus, false), !cross),
            Choice::new("Across directories", params.link(query, defaults, focus, true), cross),
        ],
    );
    let none = rows.is_empty().then(|| {
        let msg = format!(
            "No files changed together in at least {}.",
            fmt::count(rep.min_support, "commit", "commits")
        );
        let e = Empty::new(msg);
        if cross {
            e
        } else {
            e.step("Include other directories", params.link(query, defaults, focus, true))
        }
    });
    CouplingSection {
        rows,
        kind,
        focus: focus.map(str::to_owned),
        all_href: params.link(query, defaults, None, cross),
        threshold: threshold(rep.max_commit_files, rep.skipped_commits, rep.min_support),
        matrix: matrix(&rep.matrix, query, defaults, params),
        none,
    }
    .render()
    .unwrap_or_default()
}

/// The visible rule: which commits are left out and how many pairs need.
fn threshold(max: usize, skipped: usize, support: usize) -> String {
    let left = if skipped == 0 {
        format!("No commit changed more than {}.", fmt::count(max, "file", "files"))
    } else {
        format!(
            "{} that changed more than {} {} left out.",
            fmt::count(skipped, "commit", "commits"),
            fmt::count(max, "file", "files"),
            if skipped == 1 { "is" } else { "are" }
        )
    };
    format!("{left} A pair needs {}.", fmt::count(support, "shared commit", "shared commits"))
}

/// The top-level directories that change together, as a shaded grid.
fn matrix(
    pairs: &[gitterrace_index::DirPair],
    q: &InsightsQuery,
    d: &Defaults,
    p: &CouplingParams,
) -> Option<MatrixFigure> {
    let mut weight: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for x in pairs {
        *weight.entry(&x.a).or_default() += x.together;
        *weight.entry(&x.b).or_default() += x.together;
    }
    let mut dirs: Vec<&str> = weight.keys().copied().collect();
    dirs.sort_by(|a, b| weight[b].cmp(&weight[a]).then_with(|| a.cmp(b)));
    dirs.truncate(MATRIX_DIRS);
    if dirs.len() < 2 {
        return None;
    }
    let name = |s: &str| if s.is_empty() { "(root)".to_owned() } else { s.to_owned() };
    let at = |a: &str, b: &str| {
        pairs
            .iter()
            .find(|x| (x.a == a && x.b == b) || (x.a == b && x.b == a))
            .map_or(0, |x| x.together)
    };
    let max = dirs.iter().flat_map(|a| dirs.iter().map(|b| at(a, b))).max().unwrap_or(0);
    let all: usize = pairs.iter().map(|x| x.together).sum();
    let across = p.link(q, d, None, true);
    let cells = |r: &str| -> Vec<Cell> {
        dirs.iter()
            .map(|c| {
                let n = if r == *c { 0 } else { at(r, c) };
                Cell {
                    n,
                    level: level(n, max),
                    href: across.clone(),
                    tip: crate::charttip::part(
                        &format!("{} and {}", name(r), name(c)),
                        n as u64,
                        all as u64,
                        crate::charttip::COMMITS,
                    ),
                }
            })
            .collect()
    };
    let m = Matrix {
        cols: dirs
            .iter()
            .map(|c| MatrixCol { label: name(c), href: q.filter.link(BASE, Dim::Path, c) })
            .collect(),
        rows: dirs
            .iter()
            .map(|r| MatrixRow {
                label: name(r),
                href: q.filter.link(BASE, Dim::Path, r),
                cells: cells(r),
            })
            .collect(),
    };
    let mut head = vec!["Directory".to_owned()];
    head.extend(m.cols.iter().map(|c| c.label.clone()));
    let table = charts::view::Table {
        head,
        rows: m
            .rows
            .iter()
            .map(|r| charts::view::Row {
                label: r.label.clone(),
                cells: r.cells.iter().map(|c| fmt::thousands(c.n)).collect(),
            })
            .collect(),
        total: None,
    };
    let top = pairs.first().map_or_else(String::new, |x| {
        let pair = format!("{} and {}", name(&x.a), name(&x.b));
        format!(" The most is {pair} with {}.", fmt::count(x.together, "commit", "commits"))
    });
    let summary = format!(
        "{} shared by top-level directories.{top}",
        fmt::count(all, "commit pair", "commit pairs")
    );
    Some(MatrixFigure { m, fig: charts::fig("Directories that change together", summary, table) })
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::Fixture;

    #[tokio::test]
    async fn the_switch_adds_pairs_across_directories_and_a_focus_narrows() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut f = Fixture::init(dir.path());
        for n in 0..4 {
            f = f
                .write("pair/x.rs", &format!("{n}\n"))
                .write("pair/z.rs", &format!("{n}\n"))
                .commit("fix: siblings");
            f = f
                .write("pair/x.rs", &format!("again {n}\n"))
                .write("other/y.rs", &format!("{n}\n"))
                .commit("fix: across");
        }
        let router = crate::app_router(f.path()).expect("router");
        let get = gitterrace_testkit::page;
        gitterrace_testkit::wait::page_until(&router, "/stats/hotspots", |b| b.contains("pair/"))
            .await;
        let (_, same) = get(&router, "/stats/hotspots?period=all").await;
        assert!(same.contains("data-focus-link"), "{same}");
        assert!(!same.contains("other directory"));
        assert!(same.contains("No commit changed more than 30 files."));
        let (_, all) = get(&router, "/stats/hotspots?period=all&cross_dir=1").await;
        assert!(all.contains("other directory"), "{all}");
        let (_, one) =
            get(&router, "/stats/hotspots?period=all&cross_dir=1&focus=other%2Fy.rs").await;
        assert!(one.contains("Changes with other/y.rs"), "{one}");
        assert!(
            !one.contains("data-path=\"pair/x.rs\"><th scope=\"row\"><a href=\"/history/pair/z.rs")
        );
    }
}
