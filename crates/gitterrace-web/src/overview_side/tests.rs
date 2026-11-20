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
    git(&p, &["worktree", "add", "-q", "-b", "wt", wt.to_str().expect("utf8")]);
    std::fs::write(p.join("a.rs"), "changed\n").expect("write");
    let repo = gitterrace_git::Repo::discover(&p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = App::configured(repo, index, env);
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    app.mark_ready();
    (dir, app)
}

#[test]
fn shows_now_recent_churn_loose_ends_and_languages() {
    let (_d, app) = fixture();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    let s = build(&app, &head, &*src, app.dirty(), fmt::now());
    assert_eq!(s.branch, "main");
    assert!(s.dirty, "the edited a.rs makes the tree dirty");
    assert_eq!((s.today, s.week), (2, 2), "the side commit is not on main");
    assert_eq!(s.last.text, "just now");
    assert!(s.spark.starts_with("<svg") && s.spark.contains("polyline"));
    assert_eq!(s.hot[0].path, "a.rs");
    assert_eq!(s.hot[0].changes, "2 changes");
    let names: Vec<_> = s.loose.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["side"], "merged, current and wt (no own commits) are not loose");
    assert_eq!(s.worktrees, 1);
    assert_eq!(s.langs[0].name, "Rust");
    assert_eq!(s.langs.len(), 2);
}

#[test]
fn old_commits_fall_out_of_today_and_churn() {
    let (_d, app) = fixture();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    let s = build(&app, &head, &*src, false, fmt::now() + 200 * DAY);
    assert_eq!((s.today, s.week), (0, 0));
    assert!(s.hot.is_empty());
    assert!(s.spark_tip.ends_with("0, 0, 0"), "{}", s.spark_tip);
}

#[test]
fn sidebar_is_built_once_per_tip_and_dirty_flag() {
    let (_d, app) = fixture();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    let first = html(&app, &head, &*src);
    let n = builds();
    assert_eq!(html(&app, &head, &*src), first);
    assert_eq!(builds(), n, "repeat must hit the memo");
    assert!(first.contains("uncommitted changes"));
    let empty = gitterrace_index::feed::MemorySource::new(&[]);
    assert_eq!(html(&app, &head, &empty), first, "a repeat never reads the source");
    assert!(first.contains("side"));
}

/// Each sidebar heading's first text, whatever tags wrap it.
fn headings(html: &str) -> Vec<&str> {
    html.split("<h3")
        .skip(1)
        .map(|h| {
            let h = h.split("</h3>").next().unwrap_or("");
            h.split('>')
                .skip(1)
                .map(|t| t.split('<').next().unwrap_or("").trim())
                .find(|t| !t.is_empty())
                .unwrap_or("")
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
    let src = crate::tests::unchecked_src(&app, &head);
    assert_eq!(
        headings(&html(&app, &head, &*src)),
        ["Now", "Loose ends", "Recent", "Churn", "Languages"]
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
    let f = f.at("2024-03-08T00:30:00+00:00").write("a", "1").commit("one");
    let f = f.at("2024-03-09T12:00:00+00:00").write("a", "2").commit("two");
    let _f = f.at("2024-03-10T00:00:00+00:00").write("a", "3").commit("three");
    let repo = gitterrace_git::Repo::discover(dir.path()).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = App::new(repo, index);
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    app.mark_ready();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    // 2024-03-10T00:00:00Z, the frozen test clock.
    let html = build(&app, &head, &*src, false, 1_710_028_800).render().expect("render");
    let pts = html
        .split("<polyline points=\"")
        .nth(1)
        .and_then(|p| p.split('"').next())
        .expect("a sparkline");
    let ys: Vec<&str> = pts.split(' ').filter_map(|p| p.split(',').nth(1)).collect();
    assert!(ys.len() >= 6, "{pts}");
    assert_ne!(ys[0], "21.0", "the first bucket is before the first commit: {pts}");
    assert!(html.contains("Recent<span class=\"muted\">, last 2 days</span>"), "{html}");
    assert!(html.contains("Commits per 6 hours, last 2 days, oldest first: 1, "), "{html}");
}

#[test]
fn the_sparkline_spans_its_column_with_a_baseline_and_a_latest_dot() {
    let mut w = [0; 12];
    w[3] = 4;
    w[11] = 2;
    let s = line(&w);
    assert!(!s.contains("width="), "CSS sizes it to the column: {s}");
    assert!(s.contains("preserveAspectRatio=\"none\""), "{s}");
    assert!(s.contains("class=\"base\"") && s.contains("class=\"dot\""), "{s}");
}

/// The sidebar's sparkline reads the index, the repository list's reads git: over one
/// repository and one window both count the same commits, merges included, and agree on
/// whether the window holds the whole history.
#[test]
fn both_sparklines_count_the_same_commits() {
    use gitterrace_testkit::git_at;
    let dir = tempfile::tempdir().expect("tempdir");
    let p = dir.path();
    git(p, &["init", "-q", "-b", "main"]);
    let mut at = 1_700_000_000_i64;
    let commit = |p: &std::path::Path, msg: &str, at: i64| {
        git_at(p, &format!("@{at} +0000"), &["commit", "-q", "--allow-empty", "-m", msg]);
    };
    for n in 0..4 {
        commit(p, &format!("feat: {n}"), at);
        at += 10 * DAY;
    }
    git(p, &["checkout", "-q", "-b", "side"]);
    commit(p, "feat: side", at);
    git(p, &["checkout", "-q", "main"]);
    commit(p, "feat: main", at + 1);
    git_at(p, &format!("@{} +0000", at + 2), &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
    let repo = gitterrace_git::Repo::discover(p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let env = crate::app::Env::new(gitterrace_config::Config::default());
    let app = App::configured(repo, index, &env);
    app.index.update(&app.repo, "HEAD", &|_| {}).expect("update");
    app.mark_ready();
    let head = app.repo.resolve("HEAD").expect("head");
    let src = crate::tests::unchecked_src(&app, &head);
    let until = at + DAY;
    for from in [at - 15 * DAY, 0] {
        let (rows, start) = spark_times(&*src, from, until);
        let mut ours: Vec<i64> = rows.iter().map(|c| c.time).collect();
        let (mut git_times, git_start) = app.repo.commit_times_since(&head, from).expect("walk");
        ours.sort_unstable();
        git_times.sort_unstable();
        assert_eq!(ours, git_times, "from {from}");
        assert_eq!(start, git_start, "from {from}");
    }
}
