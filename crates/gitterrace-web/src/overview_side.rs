//! The overview's sidebar: what the working tree holds now, how recent work went, which files
//! keep churning, which branches and worktrees were left behind, and the languages at the tip.
//! Rendered once per (tip, index generation, dirty flag) through the page memo, so a warm
//! overview costs one lookup.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use askama::Template;
use gitterrace_index::feed::{CommitRange, InsightsSource, Rank};

use crate::{
    app::App,
    fmt::{self, enc_path, enc_seg, href},
};

const DAY: i64 = 86_400;
const WEEKS: usize = 12;
const CHURN_DAYS: i64 = 30;
const TOP_CHURN: usize = 3;
const TOP_LOOSE: usize = 3;
const TOP_LANGS: usize = 4;
const SPARK_W: f64 = 120.0;
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
    pub(crate) when: String,
}

pub(crate) struct Lang {
    pub(crate) name: String,
    pub(crate) pct: String,
    pub(crate) width: String,
    pub(crate) class: String,
}

/// Everything the sidebar shows, as display strings.
#[derive(Template)]
#[template(path = "_side.html")]
pub(crate) struct Side {
    pub(crate) branch: String,
    pub(crate) branch_href: String,
    pub(crate) dirty: bool,
    pub(crate) worktree_href: String,
    pub(crate) today: usize,
    pub(crate) week: usize,
    pub(crate) last: String,
    pub(crate) spark: String,
    pub(crate) spark_tip: String,
    pub(crate) hot: Vec<Hot>,
    pub(crate) hot_href: String,
    pub(crate) loose: Vec<Loose>,
    pub(crate) loose_more: String,
    pub(crate) worktrees: usize,
    pub(crate) worktrees_href: String,
    pub(crate) refs_href: String,
    pub(crate) langs: Vec<Lang>,
    pub(crate) langs_href: String,
}

/// The sidebar's HTML for `head` over `src`, from the memo when the tip, the index and the
/// working tree are as last time.
pub(crate) fn html(a: &App, head: &str, src: &dyn InsightsSource) -> String {
    let dirty = !a.public() && a.dirty();
    let key = format!(
        "side\0{}\0{head}\0{}\0{dirty}",
        fmt::base(),
        a.index.generation()
    );
    if let Some(h) = a.memo_get(&key) {
        return (*h).clone();
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
    let offset = i64::from(latest.map_or(0, |c| c.offset));
    // Days start at local midnight of the latest commit's zone: the author's day, not UTC's.
    let today_start = (now + offset).div_euclid(DAY) * DAY - offset;
    let from = today_start - (WEEKS as i64 * 7 - 1) * DAY;
    let churn_from = now - CHURN_DAYS * DAY;
    let mut weeks = [0usize; WEEKS];
    let (mut today, mut week) = (0, 0);
    let mut recent: Vec<Rank> = Vec::new();
    for c in src.commits_in(CommitRange::Time(from..now + DAY)) {
        let back = usize::try_from((today_start + DAY - 1 - c.time).div_euclid(7 * DAY))
            .unwrap_or(0)
            .min(WEEKS - 1);
        weeks[WEEKS - 1 - back] += 1;
        today += usize::from(c.time >= today_start);
        week += usize::from(c.time >= today_start - 6 * DAY);
        if c.time >= churn_from && !c.merge {
            recent.push(c.rank);
        }
    }
    let mut per: HashMap<_, usize> = HashMap::new();
    for (_, rows) in src.rows_for(recent) {
        for r in rows {
            *per.entry(r.path).or_default() += 1;
        }
    }
    let mut per: Vec<_> = per.into_iter().collect();
    per.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    let hot = per
        .into_iter()
        .filter_map(|(p, n)| Some((src.path(p)?, n)))
        .take(TOP_CHURN)
        .map(|(p, n)| Hot {
            name: p.rsplit('/').next().unwrap_or(&p).to_owned(),
            href: href(&format!("/history/{}", enc_path(&p))),
            changes: fmt::count(n, "change", "changes"),
            path: p.to_string(),
        })
        .collect();
    let (loose, loose_total) = loose_ends(a, head, branch.as_deref(), now);
    let worktrees = a.repo.linked_worktrees().map_or(0, |w| w.len());
    Side {
        branch_href: branch
            .as_deref()
            .map_or_else(String::new, |b| href(&format!("/log/{}", enc_seg(b)))),
        branch: branch.unwrap_or_else(|| "detached HEAD".to_owned()),
        dirty,
        worktree_href: href("/worktree"),
        today,
        week,
        last: latest.map_or_else(String::new, |c| fmt::ago(c.time, now)),
        spark: spark(&weeks),
        spark_tip: format!(
            "Commits per week, last {}: {}",
            fmt::count(WEEKS, "week", "weeks"),
            weeks.map(|w| w.to_string()).join(", ")
        ),
        hot,
        hot_href: href("/stats/hotspots"),
        loose_more: match loose_total.saturating_sub(loose.len()) {
            0 => String::new(),
            n => format!("{n} more"),
        },
        loose,
        worktrees,
        worktrees_href: href("/worktrees"),
        refs_href: href("/refs"),
        langs: langs(a, head),
        langs_href: href("/stats/languages"),
    }
}

/// Local branches with commits HEAD lacks, newest tip first, and how many there are.
fn loose_ends(a: &App, head: &str, current: Option<&str>, now: i64) -> (Vec<Loose>, usize) {
    let times: HashMap<String, i64> = a
        .repo
        .refs()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.kind == gitterrace_git::RefKind::Branch)
        .map(|r| (r.name, r.time))
        .collect();
    let mut open: Vec<_> = a
        .repo
        .branch_status(head)
        .unwrap_or_default()
        .into_iter()
        .filter(|b| !b.merged() && Some(b.name.as_str()) != current)
        .map(|b| (times.get(&b.name).copied().unwrap_or(0), b))
        .collect();
    open.sort_by(|x, y| y.0.cmp(&x.0).then(x.1.name.cmp(&y.1.name)));
    let total = open.len();
    let base = enc_seg(current.unwrap_or("HEAD"));
    let rows = open
        .into_iter()
        .take(TOP_LOOSE)
        .map(|(t, b)| Loose {
            href: href(&format!("/compare?base={base}&head={}", enc_seg(&b.name))),
            tip: format!(
                "{} ahead, {} behind",
                fmt::count(b.ahead, "commit", "commits"),
                b.behind
            ),
            when: fmt::ago(t, now),
            name: b.name,
        })
        .collect();
    (rows, total)
}

/// The top languages by lines at the tip, with `Other` for the rest.
fn langs(a: &App, head: &str) -> Vec<Lang> {
    let mut by: HashMap<String, u64> = HashMap::new();
    for f in a.head_lines(head).iter() {
        *by.entry(gitterrace_index::language(&f.path)).or_default() += u64::from(f.lines);
    }
    let total: u64 = by.values().sum();
    if total == 0 {
        return Vec::new();
    }
    let mut v: Vec<_> = by.into_iter().filter(|(_, n)| *n > 0).collect();
    v.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    if v.len() > TOP_LANGS {
        let rest: u64 = v.drain(TOP_LANGS..).map(|(_, n)| n).sum();
        v.push(("Other".to_owned(), rest));
    }
    v.into_iter()
        .enumerate()
        .map(|(i, (name, n))| {
            #[expect(clippy::cast_precision_loss, reason = "counts far below 2^52")]
            let share = n as f64 * 100.0 / total as f64;
            Lang {
                pct: if share < 1.0 {
                    "<1%".to_owned()
                } else {
                    format!("{share:.0}%")
                },
                width: format!("{share:.2}"),
                class: format!("c{}", i + 1),
                name,
            }
        })
        .collect()
}

/// A polyline over the weekly counts, scaled to the busiest week.
#[expect(clippy::cast_precision_loss, reason = "counts far below 2^52")]
fn spark(weeks: &[usize; WEEKS]) -> String {
    let max = weeks.iter().copied().max().unwrap_or(0).max(1) as f64;
    let step = SPARK_W / (WEEKS - 1) as f64;
    let pts: Vec<String> = weeks
        .iter()
        .enumerate()
        .map(|(i, &n)| {
            let y = (n as f64 / max).mul_add(-(SPARK_H - 2.0), SPARK_H - 1.0);
            format!("{:.1},{y:.1}", i as f64 * step)
        })
        .collect();
    format!(
        "<svg class=\"spark s1\" viewBox=\"0 0 {SPARK_W} {SPARK_H}\" width=\"{SPARK_W}\" height=\"{SPARK_H}\" aria-hidden=\"true\"><polyline points=\"{}\"/></svg>",
        pts.join(" ")
    )
}

#[cfg(test)]
pub(crate) fn builds() -> usize {
    BUILDS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;

    use super::*;

    /// A repo with an unmerged branch, a linked worktree and a dirty working tree.
    fn fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("r");
        std::fs::create_dir(&p).expect("mkdir");
        git(&p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.rs"), "fn a() {}\n").expect("write");
        std::fs::write(p.join("b.py"), "x = 1\ny = 2\n").expect("write");
        git(&p, &["add", "."]);
        git(&p, &["commit", "-q", "-m", "feat: one"]);
        std::fs::write(p.join("a.rs"), "fn a() {}\nfn b() {}\nfn d() {}\n").expect("write");
        git(&p, &["commit", "-qam", "fix: two"]);
        git(&p, &["branch", "merged"]);
        git(&p, &["checkout", "-q", "-b", "side"]);
        std::fs::write(p.join("c.rs"), "fn c() {}\n").expect("write");
        git(&p, &["add", "."]);
        git(&p, &["commit", "-q", "-m", "feat: side"]);
        git(&p, &["checkout", "-q", "main"]);
        let wt = dir.path().join("wt");
        git(
            &p,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "wt",
                wt.to_str().expect("utf8"),
            ],
        );
        std::fs::write(p.join("a.rs"), "changed\n").expect("write");
        let repo = gitterrace_git::Repo::discover(&p).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        (dir, app)
    }

    #[test]
    fn shows_now_recent_churn_loose_ends_and_languages() {
        let (_d, app) = fixture();
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        let s = build(&app, &head, &*src, app.dirty(), fmt::now());
        assert_eq!(s.branch, "main");
        assert!(s.dirty, "the edited a.rs makes the tree dirty");
        assert_eq!((s.today, s.week), (2, 2), "the side commit is not on main");
        assert_eq!(s.last, "just now");
        assert!(s.spark.starts_with("<svg") && s.spark.contains("polyline"));
        assert_eq!(s.hot[0].path, "a.rs");
        assert_eq!(s.hot[0].changes, "2 changes");
        let names: Vec<_> = s.loose.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            ["side"],
            "merged, current and wt (no own commits) are not loose"
        );
        assert_eq!(s.worktrees, 1);
        assert_eq!(s.langs[0].name, "Rust");
        assert_eq!(s.langs.len(), 2);
    }

    #[test]
    fn old_commits_fall_out_of_today_and_churn() {
        let (_d, app) = fixture();
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        let s = build(&app, &head, &*src, false, fmt::now() + 200 * DAY);
        assert_eq!((s.today, s.week), (0, 0));
        assert!(s.hot.is_empty());
        assert!(s.spark_tip.ends_with("0, 0, 0"));
    }

    #[test]
    fn sidebar_is_built_once_per_tip_and_dirty_flag() {
        let (_d, app) = fixture();
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        let first = html(&app, &head, &*src);
        let n = builds();
        assert_eq!(html(&app, &head, &*src), first);
        assert_eq!(builds(), n, "repeat must hit the memo");
        assert!(first.contains("uncommitted changes"));
        assert!(first.contains("side"));
    }
}
