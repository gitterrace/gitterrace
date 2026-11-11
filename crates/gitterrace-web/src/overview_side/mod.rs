//! The overview's sidebar: what the working tree holds now, how recent work went, which files
//! keep churning, which branches and worktrees were left behind, and the languages at the tip.
//! Rendered once per (tip, index generation, dirty flag) through the page memo, so a warm
//! overview costs one lookup.

mod lists;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use askama::Template;
use gitterrace_config::Feature;
use gitterrace_index::feed::{CommitRange, InsightsSource, Rank, local_day_of};

use self::lists::{langs, loose_ends, worktree_count};

use crate::{
    app::App,
    fmt::{self, enc_seg, href},
    spark,
};

const DAY: i64 = 86_400;
/// The sidebar's longest sparkline window.
/// Not configurable: a chart definition (the span a sparkline shows), not a resource limit.
const SPARK_CAP: i64 = 12 * spark::WEEK;
const CHURN_DAYS: i64 = 30;
const TOP_CHURN: usize = 3;
const SPARK_H: f64 = 24.0;

/// Builds of the sidebar, so a test can tell a memo hit from a rebuild.
static BUILDS: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Hot {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) changes: String,
}

pub(crate) struct Loose {
    pub(crate) name: String,
    pub(crate) href: String,
    pub(crate) tip: String,
    pub(crate) when: crate::when::When,
}

pub(crate) struct Lang {
    pub(crate) name: String,
    pub(crate) pct: String,
    /// Space separated classes: the colour and the `w<N>` width step (`--w` in app.css).
    pub(crate) class: String,
}

/// Everything the sidebar shows, as display strings.
#[derive(Template)]
#[template(path = "side.html")]
pub(crate) struct Side {
    /// `display.sidebar`: which sections show, in order.
    pub(crate) order: Vec<String>,
    pub(crate) branch: String,
    pub(crate) branch_href: String,
    pub(crate) dirty: bool,
    pub(crate) worktree_href: String,
    pub(crate) today: usize,
    pub(crate) week: usize,
    pub(crate) last: crate::when::When,
    pub(crate) spark: String,
    pub(crate) spark_tip: String,
    /// The sparkline's window, "9 days", beside the heading.
    pub(crate) spark_span: String,
    pub(crate) hot: Vec<Hot>,
    pub(crate) hot_href: String,
    pub(crate) loose: Vec<Loose>,
    pub(crate) loose_more: String,
    pub(crate) worktrees: usize,
    pub(crate) worktrees_href: String,
    pub(crate) refs_href: String,
    pub(crate) langs: Vec<Lang>,
    pub(crate) langs_href: String,
    pub(crate) insights_href: String,
}

/// The sidebar's HTML for `head` over `src`, from the memo when the tip, the index and the
/// working tree are as last time.
pub(crate) fn html(a: &App, head: &str, src: &dyn InsightsSource) -> String {
    let dirty = !a.public() && crate::features::on(Feature::Worktree) && a.dirty();
    // The section list joins the key: a Live reload of `display.sidebar` must not be
    // answered with the old sidebar.
    let key = format!(
        "side\0{}\0{}\0{head}\0{}\0{dirty}\0{}",
        fmt::base(),
        fmt::selected(),
        a.index.generation(),
        a.cfg().display.sidebar.join(",")
    );
    if let Some(h) = a.memo_get(&key) {
        return h;
    }
    let out = build(a, head, src, dirty, fmt::now())
        .render()
        .unwrap_or_else(|e| {
            tracing::warn!("sidebar not rendered: {e}");
            String::new()
        });
    a.memo_put(key, std::sync::Arc::new(out.clone()));
    out
}

pub(crate) fn build(a: &App, head: &str, src: &dyn InsightsSource, dirty: bool, now: i64) -> Side {
    BUILDS.fetch_add(1, Ordering::Relaxed);
    let branch = a.repo.head_branch();
    let latest = src
        .len()
        .checked_sub(1)
        .and_then(|i| src.commit(Rank(u32::try_from(i).unwrap_or(u32::MAX))));
    let offset = latest.map_or(0, |c| c.offset);
    // Days start at local midnight of the latest commit's zone. The spark and today count
    // every commit by the wall clock, as the overview does. The hot files count what the
    // hotspots page counts (no merges, bots by the option).
    let today_start = local_day_of(now, offset) * DAY - i64::from(offset);
    let from = (now - SPARK_CAP).min(today_start - 6 * DAY);
    let churn_from = now - CHURN_DAYS * DAY;
    let (window, start) = spark_times(src, from, now + DAY);
    let mut times = Vec::new();
    let (mut today, mut week) = (0, 0);
    let mut recent: Vec<Rank> = Vec::new();
    // Who counts is read from HEAD's shared tables: a fresh set rebuilt the people of the
    // whole history on every cold overview.
    let counts = crate::insights::shared_on(a, src, head, a.defaults().into_options());
    for c in window {
        times.push(c.time);
        today += usize::from(c.time >= today_start);
        week += usize::from(c.time >= today_start - 6 * DAY);
        if c.time >= churn_from && counts.counted_row(&c) {
            recent.push(c.rank);
        }
    }
    let hot = hot_files(a, head, src, recent);
    let (loose, loose_total) = loose_ends(a, head, branch.as_deref(), now);
    let worktrees = worktree_count(a);
    let series = spark::series(
        times.iter().copied(),
        start.or(Some(from - SPARK_CAP)),
        spark::End::local(now, offset),
        SPARK_CAP,
    );
    Side {
        order: a.cfg().display.sidebar.clone(),
        branch_href: branch
            .as_deref()
            .map_or_else(String::new, |b| href(&format!("/log/{}", enc_seg(b)))),
        branch: branch.unwrap_or_else(|| "detached HEAD".to_owned()),
        dirty,
        worktree_href: href("/worktree"),
        today,
        week,
        last: latest.map_or_else(crate::when::When::none, |c| fmt::ago(c.time, now)),
        spark: if series.counts.iter().any(|&n| n > 0) {
            line(&series.counts)
        } else {
            String::new()
        },
        spark_span: series.span.clone(),
        spark_tip: format!(
            "{}, oldest first: {}",
            series.label(),
            series
                .counts
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        hot,
        hot_href: crate::features::page_href("hotspots"),
        loose_more: match loose_total.saturating_sub(loose.len()) {
            0 => String::new(),
            n => format!("{n} more"),
        },
        loose,
        worktrees,
        worktrees_href: href("/worktrees"),
        refs_href: href("/refs"),
        langs: langs(a, head),
        langs_href: crate::features::page_href("languages"),
        insights_href: crate::features::insights_home()
            .map(href)
            .unwrap_or_default(),
    }
}

/// The most changed files among the `recent` commits, which the caller picked by the counted
/// rule, so a file's "N changes" is its hotspots row over the same days.
fn hot_files(a: &App, head: &str, src: &dyn InsightsSource, recent: Vec<Rank>) -> Vec<Hot> {
    let mut per: HashMap<_, usize> = HashMap::new();
    for (_, rows) in src.rows_for(recent) {
        for r in rows {
            *per.entry(r.path).or_default() += 1;
        }
    }
    let filter = path_filter(a, head);
    let mut per: Vec<_> = per.into_iter().collect();
    per.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    per.into_iter()
        .filter_map(|(p, n)| Some((src.path(p)?, n)))
        .filter(|(p, _)| !filter.skips(p))
        .take(TOP_CHURN)
        .map(|(p, n)| Hot {
            name: short_name(&p),
            href: crate::fmt::file_link(&p),
            changes: fmt::count(n, "change", "changes"),
            path: p.to_string(),
        })
        .collect()
}
/// The file name with its parent: the parent keeps the many `lib.rs` and `mod.rs` apart, the
/// tooltip has the rest.
fn short_name(p: &str) -> String {
    let mut parts: Vec<_> = p.rsplitn(3, '/').take(2).collect();
    parts.reverse();
    parts.join("/")
}

/// The commits of the window `from..until` and when the history began: the oldest commit
/// time when every commit is inside the window, else `None` (the window is capped). The same
/// contract as `Repo::commit_times_since`, which the repository list reads from git, so the
/// two sparklines of one repository count the same commits. Ranks are not in time order
/// (several roots, skewed clocks), so not rank 0: one probe for any commit older than `from`,
/// which stops at the first hit.
pub(crate) fn spark_times(
    src: &dyn InsightsSource,
    from: i64,
    until: i64,
) -> (Vec<gitterrace_index::feed::CommitRow>, Option<i64>) {
    let window: Vec<_> = src.commits_in(CommitRange::Time(from..until)).collect();
    let capped = src
        .commits_in(CommitRange::Time(i64::MIN..from))
        .next()
        .is_some();
    let start = (!capped)
        .then(|| window.iter().map(|c| c.time).min())
        .flatten();
    (window, start)
}

pub(crate) fn path_filter(a: &App, head: &str) -> gitterrace_index::PathFilter {
    let opts = a.defaults().into_options();
    let ignore = opts.respect_ignore.then(|| a.ignore_set(head)).flatten();
    gitterrace_index::PathFilter::new(&opts, ignore)
}

/// A polyline over the bucket counts, scaled to the busiest bucket, over a baseline, with a dot
/// on the latest bucket. One unit per bucket across, so CSS stretches it to the column's width;
/// the strokes keep their width (`vector-effect`) and the dot stays round (a zero-length
/// path with round caps).
fn line(counts: &[u32]) -> String {
    let max = f64::from(counts.iter().copied().max().unwrap_or(0).max(1));
    let y = |n: u32| (f64::from(n) / max).mul_add(-(SPARK_H - 6.0), SPARK_H - 3.0);
    // A repository under an hour old has one bucket: drawn flat across, not as a dot.
    let one = [counts.first().copied().unwrap_or(0); 2];
    let counts = if counts.len() < 2 { &one[..] } else { counts };
    let pts: Vec<String> = counts
        .iter()
        .enumerate()
        .map(|(i, &n)| format!("{i},{:.1}", y(n)))
        .collect();
    let last = counts.len() - 1;
    format!(
        "<svg class=\"spark s1\" viewBox=\"0 0 {last} {SPARK_H}\" preserveAspectRatio=\"none\" \
         aria-hidden=\"true\"><path class=\"base\" d=\"M0 {b}H{last}\"/><polyline \
         points=\"{}\"/><path class=\"dot\" d=\"M{last} {:.1}h0\"/></svg>",
        pts.join(" "),
        y(counts.last().copied().unwrap_or(0)),
        b = SPARK_H - 0.5,
    )
}

#[cfg(test)]
pub(crate) fn builds() -> usize {
    BUILDS.load(Ordering::Relaxed)
}
