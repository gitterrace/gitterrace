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
    spark,
};

const DAY: i64 = 86_400;
/// The sidebar's longest sparkline window.
const SPARK_CAP: i64 = 12 * spark::WEEK;
const CHURN_DAYS: i64 = 30;
const TOP_CHURN: usize = 3;
const TOP_LOOSE: usize = 3;
const TOP_LANGS: usize = 4;
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
    pub(crate) last: String,
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
}

/// The sidebar's HTML for `head` over `src`, from the memo when the tip, the index and the
/// working tree are as last time.
pub(crate) fn html(a: &App, head: &str, src: &dyn InsightsSource) -> String {
    let dirty = !a.public() && a.dirty();
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

/// Merge commits reachable from `head`. A pass over every commit (half a second on
/// rust-lang/rust), so counted once per (tip, index generation).
pub(crate) fn merges(a: &App, head: &str, src: &dyn InsightsSource) -> usize {
    let key = format!("merges\0{}\0{head}\0{}", fmt::base(), a.index.generation());
    if let Some(n) = a.memo_get(&key).and_then(|h| h.parse().ok()) {
        return n;
    }
    let n = src.commits_in(CommitRange::All).filter(|c| c.merge).count();
    a.memo_put(key, std::sync::Arc::new(n.to_string()));
    n
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
    let from = (now - SPARK_CAP).min(today_start - 6 * DAY);
    let churn_from = now - CHURN_DAYS * DAY;
    let mut times = Vec::new();
    let (mut today, mut week) = (0, 0);
    let mut recent: Vec<Rank> = Vec::new();
    for c in src.commits_in(CommitRange::Time(from..now + DAY)) {
        times.push(c.time);
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
    let filter = path_filter(a, head);
    let mut per: Vec<_> = per.into_iter().collect();
    per.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    let hot = per
        .into_iter()
        .filter_map(|(p, n)| Some((src.path(p)?, n)))
        .filter(|(p, _)| !filter.skips(p))
        .take(TOP_CHURN)
        .map(|(p, n)| Hot {
            // The parent keeps the many `lib.rs` and `mod.rs` apart; the tooltip has the rest.
            name: p
                .rsplitn(3, '/')
                .take(2)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("/"),
            href: href(&format!("/history/{}", enc_path(&p))),
            changes: fmt::count(n, "change", "changes"),
            path: p.to_string(),
        })
        .collect();
    let (loose, loose_total) = loose_ends(a, head, branch.as_deref(), now);
    let worktrees = a.repo.linked_worktrees().map_or(0, |w| w.len());
    let series = spark::series(
        times.iter().copied(),
        first_time(src, from, &times),
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
        last: latest.map_or_else(String::new, |c| fmt::ago(c.time, now)),
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
    // The insights filter (excludes, generated, ignored) and only named languages: a bar of
    // `.test` and `.prebuilt` says nothing about the code.
    let filter = path_filter(a, head);
    let mut by: HashMap<String, u64> = HashMap::new();
    for f in a.head_lines(head).iter().filter(|f| !filter.skips(&f.path)) {
        let l = gitterrace_index::language(&f.path);
        if l.starts_with('.') || l.starts_with('(') {
            continue;
        }
        *by.entry(l).or_default() += u64::from(f.lines);
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

/// When the history began, as far as the window cares: the oldest commit time when every
/// commit is inside `from..`, else a time before the cap, so the window is capped. Ranks are
/// not in time order (several roots, skewed clocks), so not rank 0: one probe for any commit
/// older than `from`, which stops at the first hit, and the window's own times otherwise.
fn first_time(src: &dyn InsightsSource, from: i64, times: &[i64]) -> Option<i64> {
    if src
        .commits_in(CommitRange::Time(i64::MIN..from))
        .next()
        .is_some()
    {
        return Some(from - SPARK_CAP);
    }
    times.iter().copied().min()
}

fn path_filter(a: &App, head: &str) -> gitterrace_index::PathFilter {
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
        "<svg class=\"spark s1\" viewBox=\"0 0 {last} {SPARK_H}\" preserveAspectRatio=\"none\" aria-hidden=\"true\"><path class=\"base\" d=\"M0 {b}H{last}\"/><polyline points=\"{}\"/><path class=\"dot\" d=\"M{last} {:.1}h0\"/></svg>",
        pts.join(" "),
        y(counts.last().copied().unwrap_or(0)),
        b = SPARK_H - 0.5,
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
        fixture_with(&crate::app::Env::new(gitterrace_config::Config::default()))
    }

    fn fixture_with(env: &crate::app::Env) -> (tempfile::TempDir, App) {
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
        let app = App::configured(repo, index, env);
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
        assert!(s.spark_tip.ends_with("0, 0, 0"), "{}", s.spark_tip);
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
        let empty = gitterrace_index::feed::MemorySource::new(&[]);
        assert_eq!(merges(&app, &head, &*src), 0);
        assert_eq!(
            html(&app, &head, &empty),
            first,
            "a repeat never reads the source"
        );
        assert!(first.contains("side"));
    }

    fn headings(html: &str) -> Vec<&str> {
        html.split("<h3>")
            .skip(1)
            .map(|h| {
                let h = h.split("</h3>").next().unwrap_or("");
                // A linked heading's text is inside its `<a>`; a plain one comes first.
                let h = if h.starts_with('<') {
                    h.split('>')
                        .nth(1)
                        .map_or(h, |x| x.split('<').next().unwrap_or(x))
                } else {
                    h.split('<').next().unwrap_or(h)
                };
                h.split(" <").next().unwrap_or(h)
            })
            .collect()
    }

    #[test]
    fn display_sidebar_picks_and_orders_the_sections_and_a_reload_applies() {
        let env = crate::app::Env::new(gitterrace_config::Config::default());
        let (_d, app) = fixture_with(&env);
        let app = std::sync::Arc::new(app);
        env.register(&app);
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        assert_eq!(
            headings(&html(&app, &head, &*src)),
            ["Now", "Recent", "Churn", "Loose ends", "Languages"]
        );
        let mut c = (*env.cfg()).clone();
        c.display.sidebar = vec!["languages".into(), "now".into()];
        c.validate().expect("valid");
        env.apply(c);
        assert_eq!(
            headings(&html(&app, &head, &*src)),
            ["Languages", "Now"],
            "a warm memo must not answer with the old list"
        );
    }

    /// A two-day-old repository: the line starts at its first commit, so its first point is
    /// above the baseline, and the heading and tooltip name the fitted window.
    #[test]
    fn a_two_day_repository_draws_no_leading_empty_buckets() {
        let dir = tempfile::tempdir().expect("tempdir");
        let f = gitterrace_testkit::Fixture::init(dir.path());
        let f = f
            .at("2024-03-08T00:30:00+00:00")
            .write("a", "1")
            .commit("one");
        let f = f
            .at("2024-03-09T12:00:00+00:00")
            .write("a", "2")
            .commit("two");
        let _f = f
            .at("2024-03-10T00:00:00+00:00")
            .write("a", "3")
            .commit("three");
        let repo = gitterrace_git::Repo::discover(dir.path()).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let app = App::new(repo, index);
        app.index
            .update(&app.repo, "HEAD", &|_| {})
            .expect("update");
        app.mark_ready();
        let head = app.repo.resolve("HEAD").expect("head");
        let src = app.source(&head).expect("source").expect("ready");
        // 2024-03-10T00:00:00Z, the frozen test clock.
        let html = build(&app, &head, &*src, false, 1_710_028_800)
            .render()
            .expect("render");
        let pts = html
            .split("<polyline points=\"")
            .nth(1)
            .and_then(|p| p.split('"').next())
            .expect("a sparkline");
        let ys: Vec<&str> = pts.split(' ').filter_map(|p| p.split(',').nth(1)).collect();
        assert!(ys.len() >= 6, "{pts}");
        assert_ne!(
            ys[0], "21.0",
            "the first bucket is before the first commit: {pts}"
        );
        assert!(
            html.contains("Recent<span class=\"muted\">, last 2 days</span>"),
            "{html}"
        );
        assert!(
            html.contains("Commits per 6 hours, last 2 days, oldest first: 1, "),
            "{html}"
        );
    }

    #[test]
    fn the_sparkline_spans_its_column_with_a_baseline_and_a_latest_dot() {
        let mut w = [0; 12];
        w[3] = 4;
        w[11] = 2;
        let s = line(&w);
        assert!(!s.contains("width="), "CSS sizes it to the column: {s}");
        assert!(s.contains("preserveAspectRatio=\"none\""), "{s}");
        assert!(
            s.contains("class=\"base\"") && s.contains("class=\"dot\""),
            "{s}"
        );
    }
}
