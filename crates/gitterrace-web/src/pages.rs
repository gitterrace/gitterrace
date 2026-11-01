//! The remaining insights pages: activity, hotspots, ownership, patterns, compare and file history.

use askama::Template;
use axum::extract::{Path, Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::Period;
use serde::Deserialize;

use crate::{
    app::App,
    charts,
    error::Result,
    filter::{Dim, FilterBar},
    fmt::{self, enc, href},
    insights::{
        InsightsQuery, Kpi, OptLink, cached, cached_keyed, head_lines, insights, period_of, scoped,
    },
    routes::Shared,
    views::{self, Chrome, Tabs},
};

const TOP_HOT: usize = 50;
const SCATTER: usize = 400;
const TOP_DIRS: usize = 40;
const TOP_PAIRS: usize = 30;

fn file_link(path: &str) -> String {
    href(&format!("/history/{}", enc(path, true)))
}

fn delta(now: u64, before: Option<u64>) -> (String, &'static str) {
    let Some(b) = before else {
        return (String::new(), "");
    };
    if b == 0 {
        return (
            if now == 0 {
                String::new()
            } else {
                "new".to_owned()
            },
            "up",
        );
    }
    #[allow(clippy::cast_precision_loss)]
    let pct = (now as f64 - b as f64) / b as f64 * 100.0;
    (format!("{pct:+.0}%"), if pct >= 0.0 { "up" } else { "dn" })
}

// ---- activity ---------------------------------------------------------------------------

struct PulseRow {
    label: &'static str,
    now: String,
    before: String,
    delta: String,
    tone: &'static str,
}

#[derive(Template)]
#[template(path = "stats.html")]
struct ActivityPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    rows: Vec<PulseRow>,
    span: String,
    compared: bool,
    unit: &'static str,
    series: String,
    heatmap: String,
}

/// The landing insights page: the period against the one before it, commits per bucket and
/// when commits happen.
pub(crate) async fn activity(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "a", raw, move |a, r, c| {
        let f = &q.filter;
        let recs = scoped(f, r);
        let d = a.defaults();
        let ins = insights(a, &recs, q.options(&d));
        let period = period_of(&q, &d, Period::All);
        let s = ins.period_summary(period);
        let prev = s.previous.as_ref();
        let row = |label, now: u64, before: Option<u64>| {
            let (d, tone) = delta(now, before);
            PulseRow {
                label,
                now: fmt::thousands(now),
                before: before.map_or_else(|| "-".to_owned(), fmt::thousands),
                delta: d,
                tone,
            }
        };
        let rows = vec![
            row(
                "Commits",
                s.current.commits as u64,
                prev.map(|p| p.commits as u64),
            ),
            row(
                "Authors",
                s.current.authors as u64,
                prev.map(|p| p.authors as u64),
            ),
            row(
                "Files changed",
                s.current.files as u64,
                prev.map(|p| p.files as u64),
            ),
            row(
                "Lines added",
                s.current.additions,
                prev.map(|p| p.additions),
            ),
            row(
                "Lines removed",
                s.current.deletions,
                prev.map(|p| p.deletions),
            ),
        ];
        let span = match s.from {
            Some(from) => format!(
                "{} to {}, against the {} days before",
                fmt::day(from),
                fmt::day(s.to - 1),
                period.days().unwrap_or(0)
            ),
            None => format!("All history up to {}", fmt::day(s.to - 1)),
        };
        let rhythm = ins.rhythm(period);
        let series: Vec<_> = rhythm
            .series
            .iter()
            .map(|p| (p.label.clone(), u64::from(p.commits)))
            .collect();
        ActivityPage {
            c,
            tabs: views::stats_tabs("a"),
            fb: f.bar("/stats", r),
            opts: q.row(&d, "/stats"),
            rows,
            span,
            compared: prev.is_some(),
            unit: rhythm.unit.noun(),
            series: charts::bars(&series),
            heatmap: charts::heatmap(&rhythm.punch_card, rhythm.punch_max),
        }
    })
    .await
}

// ---- hotspots --------------------------------------------------------------------------

struct HotRow {
    path: String,
    href: String,
    language: String,
    lines: String,
    changes: String,
    authors: String,
    last: String,
    score: String,
    pct: u32,
}

#[derive(Template)]
#[template(path = "hotspots.html")]
struct HotspotsPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    scatter: String,
    rows: Vec<HotRow>,
}

pub(crate) async fn hotspots(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "ho", raw, move |a, r, c| {
        let f = &q.filter;
        let recs = scoped(f, r);
        let d = a.defaults();
        let ins = insights(a, &recs, q.options(&d));
        let files = head_lines(a);
        let rep = ins.hotspots(&files, period_of(&q, &d, Period::All));
        let max = rep.files.first().map_or(0.0, |h| h.score);
        let dots: Vec<_> = rep
            .files
            .iter()
            .take(SCATTER)
            .map(|h| charts::Dot {
                x: h.changes as u64,
                y: u64::from(h.lines),
                size: h.authors as u64,
                href: file_link(&h.path),
                title: format!(
                    "{}: {} changes, {} lines, {} authors",
                    h.path,
                    h.changes,
                    fmt::thousands(h.lines),
                    h.authors
                ),
            })
            .collect();
        let rows = rep
            .files
            .iter()
            .take(TOP_HOT)
            .map(|h| HotRow {
                path: h.path.clone(),
                href: file_link(&h.path),
                language: h.language.clone(),
                lines: fmt::thousands(h.lines),
                changes: fmt::thousands(h.changes),
                authors: fmt::thousands(h.authors),
                last: fmt::day(h.last_change),
                score: fmt::thousands(h.score.round() as u64),
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                pct: if max > 0.0 {
                    (h.score / max * 100.0).max(1.0) as u32
                } else {
                    0
                },
            })
            .collect();
        HotspotsPage {
            c,
            tabs: views::stats_tabs("ho"),
            fb: f.bar("/stats/hotspots", r),
            opts: q.row(&d, "/stats/hotspots"),
            kpis: vec![
                Kpi("Files changed", fmt::thousands(rep.files.len())),
                Kpi("Changes", fmt::thousands(rep.total_changes)),
                Kpi(
                    "Top tenth takes",
                    format!("{:.0}%", rep.concentration * 100.0),
                ),
                Kpi(
                    "Single-author files",
                    fmt::thousands(rep.single_author_files),
                ),
            ],
            scatter: charts::scatter(&dots, "changes"),
            rows,
        }
    })
    .await
}

// ---- ownership -------------------------------------------------------------------------

struct DirRow {
    path: String,
    href: String,
    lines: String,
    files: String,
    owners: Vec<(String, String)>,
    bus: usize,
}

#[derive(Template)]
#[template(path = "ownership.html")]
struct OwnershipPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    kpis: Vec<Kpi>,
    rows: Vec<DirRow>,
}

pub(crate) async fn ownership(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "ow", raw, move |a, r, c| {
        let f = &q.filter;
        let recs = scoped(f, r);
        let d = a.defaults();
        let ins = insights(a, &recs, q.options(&d));
        let files = head_lines(a);
        let rep = ins.ownership(
            &files,
            period_of(&q, &d, Period::All),
            gitterrace_api::OWNERSHIP_DEPTH,
        );
        let row = |d: &gitterrace_index::DirOwnership, name: String| DirRow {
            href: f.link("/stats/ownership", Dim::Path, &d.path),
            path: name,
            lines: fmt::thousands(d.lines),
            files: fmt::thousands(d.files),
            owners: d
                .owners
                .iter()
                .take(3)
                .map(|o| (o.name.clone(), format!("{:.0}%", o.share * 100.0)))
                .collect(),
            bus: d.bus_factor,
        };
        let mut rows = vec![row(&rep.overall, "(whole repository)".to_owned())];
        rows.extend(
            rep.dirs
                .iter()
                .take(TOP_DIRS)
                .map(|d| row(d, d.path.clone())),
        );
        OwnershipPage {
            c,
            tabs: views::stats_tabs("ow"),
            fb: f.bar("/stats/ownership", r),
            opts: q.row(&d, "/stats/ownership"),
            kpis: vec![
                Kpi("Bus factor", fmt::thousands(rep.overall.bus_factor)),
                Kpi("Solo lines", fmt::thousands(rep.solo_lines)),
                Kpi("Stale lines", fmt::thousands(rep.stale_lines)),
                Kpi(
                    "Directories at one",
                    fmt::thousands(rep.dirs_with_bus_factor_one),
                ),
            ],
            rows,
        }
    })
    .await
}

// ---- patterns --------------------------------------------------------------------------

struct PairRow {
    a: String,
    a_href: String,
    b: String,
    b_href: String,
    together: String,
    degree: String,
}

struct ReleaseRow {
    name: String,
    href: String,
    when: String,
    commits: String,
    days: String,
}

#[derive(Template)]
#[template(path = "patterns.html")]
struct PatternsPage {
    c: Chrome,
    tabs: Tabs,
    fb: FilterBar,
    opts: Vec<OptLink>,
    pairs: Vec<PairRow>,
    age: String,
    sizes: String,
    releases: Vec<ReleaseRow>,
    cadence: String,
}

pub(crate) async fn patterns(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<InsightsQuery>,
) -> Result<Html<String>> {
    cached(&app, "pa", raw, move |a, r, c| {
        let f = &q.filter;
        let recs = scoped(f, r);
        let d = a.defaults();
        let ins = insights(a, &recs, q.options(&d));
        let files = head_lines(a);
        let period = period_of(&q, &d, Period::All);
        let pairs = ins
            .coupling(&files, period, 3, TOP_PAIRS)
            .into_iter()
            .map(|p| PairRow {
                a_href: file_link(&p.a),
                b_href: file_link(&p.b),
                a: p.a,
                b: p.b,
                together: fmt::thousands(p.together),
                degree: format!("{:.0}%", p.degree * 100.0),
            })
            .collect();
        let age: Vec<_> = ins
            .code_age(&files)
            .buckets
            .into_iter()
            .map(|b| (b.year.to_string(), b.lines))
            .collect();
        let sizes: Vec<_> = ins
            .commit_sizes()
            .into_iter()
            .map(|b| (b.label, b.commits as u64))
            .collect();
        let tags = gitterrace_index::release_tags(&a.repo.refs().unwrap_or_default());
        let cad = ins.cadence(&tags);
        let cadence = match (cad.median_days_between, cad.days_since_last) {
            (Some(m), Some(d)) => {
                format!("Median {m:.0} days between releases, {d} days since the last.")
            }
            (None, Some(d)) => format!("{d} days since the last release."),
            _ => "No release tags.".to_owned(),
        };
        let releases = cad
            .releases
            .iter()
            .rev()
            .take(15)
            .map(|rl| ReleaseRow {
                href: href(&format!("/log/{}", enc(&rl.name, false))),
                name: rl.name.clone(),
                when: fmt::day(rl.time),
                commits: fmt::thousands(rl.commits_since),
                days: rl
                    .days_since
                    .map_or_else(|| "-".to_owned(), |d| d.to_string()),
            })
            .collect();
        PatternsPage {
            c,
            tabs: views::stats_tabs("pa"),
            fb: f.bar("/stats/patterns", r),
            opts: q.row(&d, "/stats/patterns"),
            pairs,
            age: charts::bars(&age),
            sizes: charts::bars(&sizes),
            releases,
            cadence,
        }
    })
    .await
}

// ---- compare ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub(crate) struct CompareQuery {
    base: Option<String>,
    head: Option<String>,
}

struct LangRow {
    language: String,
    added: String,
    removed: String,
    net: String,
    tone: &'static str,
}

struct Listed {
    href: String,
    sha: String,
    subject: String,
}

struct Result2 {
    kpis: Vec<Kpi>,
    langs: Vec<LangRow>,
    commits: Vec<Listed>,
    more: usize,
    diff_href: String,
}

#[derive(Template)]
#[template(path = "compare.html")]
struct ComparePage {
    c: Chrome,
    tabs: Tabs,
    base: String,
    head: String,
    tags: Vec<(String, String)>,
    error: Option<String>,
    result: Option<Result2>,
}

const COMPARE_LIST: usize = 200;

pub(crate) async fn compare(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<CompareQuery>,
) -> Result<Html<String>> {
    let (base, head) = (
        q.base.unwrap_or_default(),
        q.head.unwrap_or_else(|| "HEAD".to_owned()),
    );
    let b2 = base.clone();
    let h2 = head.clone();
    cached(&app, "cm", raw, move |a, r, c| {
        let tags = a
            .repo
            .refs()
            .unwrap_or_default()
            .into_iter()
            .filter(|x| x.kind == gitterrace_git::RefKind::Tag)
            .map(|x| {
                (
                    x.name.clone(),
                    href(&format!("/compare?base={}&head=HEAD", enc(&x.name, false))),
                )
            })
            .take(12)
            .collect();
        let mut error = None;
        let mut result = None;
        if !b2.is_empty() {
            match (a.repo.resolve(&b2), a.repo.resolve(&h2)) {
                (Ok(bi), Ok(hi)) => {
                    let ins = insights(a, r, a.defaults().into_options());
                    let cmp = ins.compare(&bi, &hi);
                    let subjects = a.subjects(&hi);
                    let net = cmp.added.abs_diff(cmp.removed);
                    let kpis = vec![
                        Kpi("Commits", fmt::thousands(cmp.commits.len())),
                        Kpi("Authors", fmt::thousands(cmp.authors)),
                        Kpi("Files changed", fmt::thousands(cmp.files_changed)),
                        Kpi("Lines added", fmt::thousands(cmp.added)),
                        Kpi("Lines removed", fmt::thousands(cmp.removed)),
                        Kpi(
                            "Net",
                            format!(
                                "{}{}",
                                if cmp.added >= cmp.removed { "+" } else { "-" },
                                fmt::thousands(net)
                            ),
                        ),
                    ];
                    let langs = cmp
                        .languages
                        .iter()
                        .map(|l| LangRow {
                            language: l.language.clone(),
                            added: fmt::thousands(l.added),
                            removed: fmt::thousands(l.removed),
                            net: format!(
                                "{}{}",
                                if l.net >= 0 { "+" } else { "-" },
                                fmt::thousands(l.net.unsigned_abs())
                            ),
                            tone: if l.net >= 0 { "add" } else { "del" },
                        })
                        .collect();
                    let commits = cmp
                        .commits
                        .iter()
                        .take(COMPARE_LIST)
                        .map(|id| Listed {
                            href: href(&format!("/commit/{id}")),
                            sha: fmt::sha_link(id),
                            subject: subjects.get(id).cloned().unwrap_or_default(),
                        })
                        .collect();
                    result = Some(Result2 {
                        kpis,
                        langs,
                        commits,
                        more: cmp.commits.len().saturating_sub(COMPARE_LIST),
                        diff_href: href(&format!("/commit/{hi}")),
                    });
                }
                _ => error = Some("One of the revisions does not exist.".to_owned()),
            }
        }
        ComparePage {
            c,
            tabs: views::stats_tabs("cm"),
            base: b2,
            head: h2,
            tags,
            error,
            result,
        }
    })
    .await
}

// ---- file history ----------------------------------------------------------------------

struct HistRow {
    href: String,
    sha: String,
    subject: String,
    author: String,
    author_href: String,
    when: String,
    lines: String,
    renamed: Option<String>,
}

#[derive(Template)]
#[template(path = "history.html")]
struct HistoryPage {
    c: Chrome,
    path: String,
    blob_href: String,
    blame_href: String,
    rows: Vec<HistRow>,
}

pub(crate) async fn history(
    State(app): Shared,
    Path(path): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Html<String>> {
    let path = path.trim_matches('/').to_owned();
    let key = format!("{path}\0{}", App::canonical_query(raw.as_deref()));
    cached_keyed(&app, "hi", key, move |a, r, c| {
        let ins = insights(a, r, a.defaults().into_options());
        let head = a.repo.resolve(&fmt::selected()).unwrap_or_default();
        let subjects = a.subjects(&head);
        let now = fmt::now();
        let rows = ins
            .file_history(&path)
            .into_iter()
            .map(|t| {
                let email = r
                    .iter()
                    .find(|x| x.id == t.commit)
                    .map_or_else(String::new, |x| x.email.clone());
                HistRow {
                    href: href(&format!("/commit/{}", t.commit)),
                    sha: fmt::sha_link(&t.commit),
                    subject: subjects.get(&t.commit).cloned().unwrap_or_default(),
                    author_href: views::author_href(&email),
                    author: t.author,
                    when: fmt::ago(t.time, now),
                    lines: fmt::thousands(t.lines),
                    renamed: (t.path != path).then_some(t.path),
                }
            })
            .collect();
        HistoryPage {
            c,
            blob_href: href(&format!(
                "/blob/{}/{}",
                fmt::selected_seg(),
                enc(&path, true)
            )),
            blame_href: href(&format!(
                "/blame/{}/{}",
                fmt::selected_seg(),
                enc(&path, true)
            )),
            path,
            rows,
        }
    })
    .await
}
