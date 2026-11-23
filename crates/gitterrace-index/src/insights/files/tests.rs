//! Unit tests for file history, hotspots and ownership.

use super::super::HeadFile;
use crate::insights::fixtures::{file, rec};
use crate::{FileStat, Insights, Options, Period};

fn head(files: &[(&str, u32)]) -> Vec<HeadFile> {
    files.iter().map(|(p, n)| HeadFile { path: (*p).into(), lines: *n }).collect()
}

fn renamed(old: &str, new: &str) -> FileStat {
    FileStat { old: Some(old.into()), ..file(new, 0, 0, 10) }
}

fn history() -> Vec<crate::CommitRecord> {
    vec![
        rec("a", "Ann", 0, vec![file("old/x.rs", 10, 0, 10), file("keep.rs", 4, 0, 4)]),
        rec("b", "Bob", 400, vec![file("old/x.rs", 2, 1, 11)]),
        rec("c", "Ann", 401, vec![renamed("old/x.rs", "new/x.rs"), file("keep.rs", 1, 1, 4)]),
        rec("d", "Ann", 402, vec![file("new/x.rs", 3, 0, 14), file("keep.rs", 1, 0, 5)]),
    ]
}

#[test]
fn ownership_is_the_same_on_every_build() {
    // Ann and Bob tie on every file; Ann is stale, Bob is not. A hash-ordered pick of the
    // top owner flipped "stale" between runs.
    let mut recs = Vec::new();
    for n in 0..40 {
        let path = format!("f{n}.rs");
        recs.push(rec(&format!("a{n}"), "Ann", 0, vec![file(&path, 5, 0, 5)]));
        recs.push(rec(&format!("b{n}"), "Bob", 900, vec![file(&path, 0, 5, 5)]));
    }
    let files: Vec<(String, u32)> = (0..40).map(|n| (format!("f{n}.rs"), 5)).collect();
    let files: Vec<(&str, u32)> = files.iter().map(|(p, l)| (p.as_str(), *l)).collect();
    let render = || {
        let ins = Insights::new(&recs, Options::default(), None);
        let r = ins.ownership(&head(&files), Period::All, 1);
        format!("{} {} {:?}", r.stale_lines, r.solo_lines, r.overall)
    };
    let first = render();
    for _ in 0..20 {
        assert_eq!(render(), first);
    }
}

#[test]
fn hotspots_follow_renames() {
    let recs = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let rep = ins.hotspots(&head(&[("new/x.rs", 14), ("keep.rs", 5)]), Period::All);
    let x = rep.files.iter().find(|f| f.path == "new/x.rs").map(|f| (f.changes, f.authors));
    assert_eq!(x, Some((4, 2)));
    assert_eq!(rep.files[0].path, "new/x.rs");
    assert!(4.0f64.mul_add(-14f64.sqrt(), rep.files[0].score).abs() < 1e-9);
    assert_eq!(rep.total_changes, 7);
    assert_eq!(rep.single_author_files, 1);
    let h = ins.file_history("new/x.rs");
    assert_eq!(h.len(), 4);
    assert_eq!(h[0].commit, "d");
}

#[test]
fn ownership_and_bus_factor() {
    let recs = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let o = ins.ownership(&head(&[("new/x.rs", 14), ("keep.rs", 5)]), Period::All, 1);
    // new/x.rs: Ann 10+0+3 = 13 changed lines, Bob 3. keep.rs: Ann only.
    let nx = o.dirs.iter().find(|d| d.path == "new").map(|d| d.owners[0].name.clone());
    assert_eq!(nx.as_deref(), Some("Ann"));
    assert_eq!(o.overall.bus_factor, 1);
    let root = o.dirs.iter().find(|d| d.path.is_empty()).map(|d| d.bus_factor);
    assert_eq!(root, Some(1));
    let total: f64 = o.overall.owners.iter().map(|x| x.share).sum();
    assert!((total - 1.0).abs() < 1e-9);
    // Ann made 13 of the 16 changed lines of new/x.rs (81%), so both files are solo.
    assert_eq!(o.solo_lines, 19);
    assert_eq!(o.stale_lines, 0);
}

#[test]
fn coupling_and_age() {
    let recs = history();
    let ins = Insights::new(&recs, Options::default(), None);
    let hf = head(&[("new/x.rs", 14), ("keep.rs", 5)]);
    let c = ins.coupling(&hf, Period::All, 2, 10);
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].together, c[0].a.as_str(), c[0].b.as_str()), (3, "keep.rs", "new/x.rs"));
    assert!((c[0].degree - 1.0).abs() < 1e-9);
    let age = ins.code_age(&hf);
    assert_eq!(age.total_lines, 19);
    assert_eq!(age.buckets.len(), 1);
    assert_eq!(age.buckets[0].year, 2025);
}

#[test]
fn bots_are_excluded_by_default() {
    let recs = vec![
        rec("a", "Ann", 0, vec![file("a.rs", 5, 0, 5)]),
        rec("b", "dependabot[bot]", 1, vec![file("a.rs", 5, 0, 10)]),
    ];
    let hf = head(&[("a.rs", 10)]);
    let ins = Insights::new(&recs, Options::default(), None);
    assert_eq!(ins.hotspots(&hf, Period::All).files[0].changes, 1);
    let with = Insights::new(&recs, Options { include_bots: true, ..Options::default() }, None);
    assert_eq!(with.hotspots(&hf, Period::All).files[0].changes, 2);
}

/// The one-pass scan over many head paths answers exactly as a postings lookup per path,
/// through renames, a name reused after it moved, and a file renamed twice.
#[test]
fn the_many_path_scan_equals_a_lookup_per_path() {
    let mut recs = history();
    recs.push(rec("e", "Bob", 500, vec![file("old/x.rs", 7, 0, 7)]));
    recs.push(rec("f", "Ann", 501, vec![renamed("new/x.rs", "z.rs")]));
    recs.push(rec("g", "Bob", 502, vec![file("z.rs", 1, 1, 14)]));
    // Padding so the scan path is taken.
    let mut names: Vec<String> =
        (0..super::PER_PATH_MAX + 5).map(|n| format!("pad/{n}.rs")).collect();
    for (i, n) in names.iter().enumerate() {
        recs.push(rec(&format!("p{i}"), "Cy", 600, vec![file(n, 1, 0, 1)]));
    }
    names.extend(["z.rs", "old/x.rs", "new/x.rs", "keep.rs", "missing.rs"].map(String::from));
    let ins = Insights::new(&recs, Options::default(), None);
    let paths: Vec<&str> = names.iter().map(String::as_str).collect();
    let scan = ins.touches_scan(&paths, |_| true);
    for (i, p) in paths.iter().enumerate() {
        let one = ins.touches_many(&[p], |_| true).pop().unwrap_or_default();
        let key = |l: &[(super::TouchRaw, crate::feed::CommitRow)]| {
            l.iter().map(|(t, c)| (t.commit, t.row, t.path, t.lines, c.rank)).collect::<Vec<_>>()
        };
        assert_eq!(key(&scan[i]), key(&one), "{p}");
    }
}

/// A source that counts postings lookups, forwarding everything else.
struct Counting<'a> {
    inner: crate::feed::MemorySource<'a>,
    lookups: std::sync::atomic::AtomicUsize,
}

impl crate::feed::InsightsSource for Counting<'_> {
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn commit(&self, r: crate::feed::Rank) -> Option<crate::feed::CommitRow> {
        self.inner.commit(r)
    }
    fn commits_in(
        &self,
        r: crate::feed::CommitRange,
    ) -> crate::feed::Stream<'_, crate::feed::CommitRow> {
        self.inner.commits_in(r)
    }
    fn rows_for(
        &self,
        c: Vec<crate::feed::Rank>,
    ) -> crate::feed::Stream<'_, (crate::feed::Rank, Vec<crate::feed::FileRow>)> {
        self.inner.rows_for(c)
    }
    fn commit_id(&self, r: crate::feed::Rank) -> Option<std::sync::Arc<str>> {
        self.inner.commit_id(r)
    }
    fn parent_ids(&self, r: crate::feed::Rank) -> Vec<std::sync::Arc<str>> {
        self.inner.parent_ids(r)
    }
    fn find_commit(&self, id: &str) -> Result<Option<crate::feed::Rank>, crate::feed::SourceError> {
        self.inner.find_commit(id)
    }
    fn agg_author_day(
        &self,
        d: std::ops::Range<i64>,
    ) -> crate::feed::Stream<'_, crate::feed::AuthorDayRow> {
        self.inner.agg_author_day(d)
    }
    fn agg_path_author(&self, p: &str) -> crate::feed::Stream<'_, crate::feed::PathAuthorRow> {
        self.inner.agg_path_author(p)
    }
    fn postings(&self, k: crate::feed::Posting) -> crate::feed::Stream<'_, crate::feed::Hit> {
        self.lookups.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.postings(k)
    }
    fn rename_edges(&self) -> crate::feed::Stream<'_, crate::feed::RenameEdge> {
        self.inner.rename_edges()
    }
    fn path(&self, id: crate::feed::PathId) -> Option<std::sync::Arc<str>> {
        self.inner.path(id)
    }
    fn find_path(&self, p: &str) -> Result<Option<crate::feed::PathId>, crate::feed::SourceError> {
        self.inner.find_path(p)
    }
    fn paths(&self) -> usize {
        self.inner.paths()
    }
    fn ident(
        &self,
        id: crate::feed::IdentId,
    ) -> Option<(std::sync::Arc<str>, std::sync::Arc<str>)> {
        self.inner.ident(id)
    }
    fn idents(&self) -> usize {
        self.inner.idents()
    }
    fn label(&self, id: crate::feed::LabelId) -> Option<std::sync::Arc<str>> {
        self.inner.label(id)
    }
    fn find_label(
        &self,
        n: &str,
    ) -> Result<Option<crate::feed::LabelId>, crate::feed::SourceError> {
        self.inner.find_label(n)
    }
    fn failure(&self) -> Option<crate::feed::SourceError> {
        self.inner.failure()
    }
}

/// Hotspots, ownership and code age over many head files read the postings a fixed number
/// of times, not once per file: a lookup per file is tens of thousands of block reads on a
/// large tree. Red on the per-file loop.
#[test]
fn file_pages_read_postings_a_fixed_number_of_times() {
    let n = 500;
    let recs: Vec<crate::CommitRecord> = (0..n)
        .map(|i| {
            rec(&format!("c{i}"), "Ann", i64::from(i), vec![file(&format!("f{i}.rs"), 1, 0, 1)])
        })
        .collect();
    let hf: Vec<HeadFile> =
        (0..n).map(|i| HeadFile { path: format!("f{i}.rs"), lines: 1 }).collect();
    let src = Counting {
        inner: crate::feed::MemorySource::new(&recs),
        lookups: std::sync::atomic::AtomicUsize::new(0),
    };
    let ins = Insights::from_source(&src, Options::default(), None);
    let read = || src.lookups.swap(0, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(ins.hotspots(&hf, Period::All).files.len(), n as usize);
    let hot = read();
    drop(ins.ownership(&hf, Period::All, 1));
    let own = read();
    drop(ins.code_age(&hf));
    let age = read();
    assert!(hot + own + age <= 3, "postings lookups: {hot} {own} {age}");
}

/// Over all history, ownership read from the path x author aggregate equals ownership
/// from the file row touches, with renames, bots, whitespace and a file nobody renamed.
#[test]
fn ownership_from_the_aggregate_equals_the_touches() {
    let mut recs = history();
    recs.push(rec("e", "dependabot[bot]", 403, vec![file("keep.rs", 9, 2, 5)]));
    // Whitespace-only lines: raw and code counts differ.
    recs.push(rec(
        "f",
        "Bob",
        404,
        vec![FileStat {
            code: Some(crate::CodeStat { additions: 1, deletions: 0, rewritten: 0 }),
            ..file("keep.rs", 6, 4, 5)
        }],
    ));
    let hf = head(&[("new/x.rs", 14), ("keep.rs", 5), ("gone.rs", 3)]);
    for (bots, ws) in [(false, false), (true, false), (false, true), (true, true)] {
        let opts = Options { include_bots: bots, count_whitespace: ws, ..Options::default() };
        let ins = Insights::new(&recs, opts, None);
        let shown: Vec<&HeadFile> = hf.iter().collect();
        let fast = ins.in_period(Period::All, |c| ins.owner_lines(&shown, c, true));
        let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
        let slow: Vec<std::collections::BTreeMap<usize, u64>> = ins
            .in_period(Period::All, |c| ins.touches_in(&names, c))
            .into_iter()
            .map(|t| {
                let mut m = std::collections::BTreeMap::new();
                for (t, c) in t {
                    *m.entry(ins.person_of(c.ident).unwrap_or(usize::MAX)).or_default() +=
                        u64::from(t.lines);
                }
                m
            })
            .collect();
        assert_eq!(fast, slow, "bots {bots} ws {ws}");
    }
}

/// A name reused after a rename: `a.rs` moved to `b.rs`, then a new `a.rs` appeared.
/// The path x author aggregate holds the old file's rows under `a.rs`, so the hybrid must
/// not read it for `a.rs` (nor `b.rs`): both answer as the row touches do. Red when the
/// split only checks whether the head name itself was renamed away.
#[test]
fn a_reused_name_takes_the_touches_not_the_aggregate() {
    let recs = vec![
        rec("a", "Ann", 0, vec![file("a.rs", 10, 0, 10)]),
        rec("b", "Ann", 10, vec![renamed("a.rs", "b.rs")]),
        rec("c", "Bob", 20, vec![file("a.rs", 3, 0, 3)]),
    ];
    let hf = head(&[("a.rs", 3), ("b.rs", 10)]);
    let ins = Insights::new(&recs, Options::default(), None);
    let shown: Vec<&HeadFile> = hf.iter().collect();
    let fast = ins.in_period(Period::All, |c| ins.owner_lines(&shown, c, true));
    let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
    let slow: Vec<std::collections::BTreeMap<usize, u64>> = ins
        .in_period(Period::All, |c| ins.touches_in(&names, c))
        .into_iter()
        .map(|t| {
            let mut m = std::collections::BTreeMap::new();
            for (t, c) in t {
                *m.entry(ins.person_of(c.ident).unwrap_or(usize::MAX)).or_default() +=
                    u64::from(t.lines);
            }
            m
        })
        .collect();
    assert_eq!(fast, slow);
    let ann = ins.person_of(crate::feed::IdentId(0)).unwrap_or(usize::MAX);
    assert!(!fast[0].contains_key(&ann), "the new a.rs has only Bob's lines");
}

/// Hotspots, ownership and coupling count a commit by its author's local day, the rule
/// [`crate::Scope::admits`] uses, not by the instant window ending one second after "now".
/// The newest commit is day 10 at noon UTC, so 1w is local days 4..=10. `late` is local
/// day 3 23:30 at -0200 (inside the instant window, outside the days): it must not count.
/// `early` is local day 4 01:00 at +1400 (before the instant window, inside the days): it
/// must count. Red on the instant window: the counts are swapped.
#[test]
fn file_reports_count_by_local_day() {
    use crate::insights::fixtures::{DAYS, MON};
    let at = |id: &str, who: &str, local: i64, offset: i32| crate::CommitRecord {
        time: MON + local - i64::from(offset),
        offset,
        ..rec(id, who, 0, vec![file("a.rs", 1, 0, 9), file("b.rs", 1, 0, 9)])
    };
    let recs = vec![
        at("late", "Lee", 3 * DAYS + 23 * 3600 + 1800, -7200),
        at("early", "Eve", 4 * DAYS + 3600, 50_400),
        rec("now", "Ann", 10, vec![file("a.rs", 1, 0, 9), file("b.rs", 1, 0, 9)]),
    ];
    let ins = Insights::new(&recs, Options::default(), None);
    let hf = head(&[("a.rs", 9), ("b.rs", 9)]);
    let hot = ins.hotspots(&hf, Period::Week);
    assert_eq!(hot.total_changes, 4, "early and now on two files");
    assert_eq!(hot.files[0].authors, 2);
    let own = ins.ownership(&hf, Period::Week, 1);
    let names: Vec<&str> = own.overall.owners.iter().map(|o| o.name.as_str()).collect();
    assert!(names.contains(&"Eve") && !names.contains(&"Lee"), "{names:?}");
    let pair = ins.coupling(&hf, Period::Week, 1, 5);
    assert_eq!(pair.first().map(|c| c.together), Some(2));
}

/// The `_in` reports over a [`crate::Counted`] equal the period reports over the same
/// unscoped history, and a path selection keeps only the files under it.
#[test]
fn file_reports_in_counted_match_the_period() {
    use crate::feed::{FixedClock, MemorySource};
    use crate::{CountContext, Scope, ScopeParams};
    let recs = history();
    let hf = head(&[("new/x.rs", 14), ("keep.rs", 5)]);
    let ins = Insights::new(&recs, Options::default(), None);
    let cx = CountContext::new(&MemorySource::new(&recs), &FixedClock(i64::MAX));
    let scope = |path: Option<&str>| {
        let mut p = ScopeParams { period: Some("1w".to_owned()), ..ScopeParams::default() };
        p.select.q = path.map(|p| format!("path:{p}"));
        Scope::from_params(&p, Options::default(), Period::All).expect("scope")
    };
    let s = scope(None);
    let c = ins.counted(&s, cx);
    assert_eq!(ins.hotspots_in(&hf, &c), ins.hotspots(&hf, Period::Week));
    assert_eq!(ins.ownership_in(&hf, &c, 1), ins.ownership(&hf, Period::Week, 1));
    assert_eq!(ins.coupling_in(&hf, &c, 1, 9), ins.coupling(&hf, Period::Week, 1, 9));
    let s = scope(Some("new"));
    let c = ins.counted(&s, cx);
    let hot = ins.hotspots_in(&hf, &c);
    assert_eq!(hot.files.len(), 1);
    assert_eq!(hot.files[0].path, "new/x.rs");
    assert_eq!(ins.coupling_in(&hf, &c, 1, 9).len(), 0);
}

/// A ranked list of people never depends on internal person ids. Ann and Yan share every
/// file of `src` equally. In one history Yan has more commits elsewhere (so a lower person
/// id), in the other Ann does. The owners must read the same, ties by name.
#[test]
fn owner_ties_do_not_follow_person_ids() {
    let build = |extra: &str| {
        let mut recs = vec![
            rec("a", "Ann", 0, vec![file("src/x.rs", 5, 0, 10)]),
            rec("z", "Yan", 1, vec![file("src/x.rs", 5, 0, 10)]),
        ];
        for n in 0..3 {
            recs.push(rec(&format!("e{n}"), extra, 2 + n, vec![file("other.rs", 1, 0, 1)]));
        }
        let ins = Insights::new(&recs, Options::default(), None);
        let o = ins.ownership(&head(&[("src/x.rs", 10)]), Period::All, 1);
        o.dirs
            .iter()
            .find(|d| d.path == "src")
            .map(|d| d.owners.iter().map(|o| o.name.clone()).collect::<Vec<_>>())
    };
    let ann = build("Ann");
    let zed = build("Yan");
    assert_eq!(ann, zed);
    assert_eq!(ann, Some(vec!["Ann".to_owned(), "Yan".to_owned()]));
}

/// A file's age is its last counted change: a merge that repeats its lines a year later, or
/// a commit the selection leaves out, never makes it younger.
#[test]
fn code_age_follows_the_membership_rule() {
    use crate::feed::{FixedClock, MemorySource};
    use crate::{CountContext, Scope, ScopeParams};
    let mut recs = vec![
        rec("a", "Ann", 0, vec![file("x.rs", 5, 0, 5)]),
        rec("m", "Ann", 400, vec![file("x.rs", 5, 0, 5)]),
        rec("b", "Bob", 410, vec![file("x.rs", 1, 0, 6)]),
    ];
    recs[1].merge = true;
    let hf = head(&[("x.rs", 6)]);
    let ins = Insights::new(&recs, Options::default(), None);
    let cx = CountContext::new(&MemorySource::new(&recs), &FixedClock(i64::MAX));
    let mut p = ScopeParams::default();
    p.select.q = Some("author:ann@x.io".to_owned());
    let s = Scope::from_params(&p, Options::default(), Period::All).expect("scope");
    let years = |r: crate::AgeReport| r.buckets.iter().map(|b| b.year).collect::<Vec<_>>();
    assert_eq!(years(ins.code_age_in(&hf, &ins.counted(&s, cx))), [2024], "Ann's own, no merge");
    assert_eq!(years(ins.code_age(&hf)), [2025], "Bob's change counts without a selection");
}

/// A file's history lists the commits its hotspots row counts: a bot's commits only when the
/// options count bots. `dependabot[bot]` bumps `Cargo.toml` three times, Ann once.
#[test]
fn file_history_counts_bots_by_the_option() {
    let mut recs = vec![rec("a", "Ann", 0, vec![file("Cargo.toml", 1, 0, 1)])];
    for d in 1..=3 {
        recs.push(rec(&format!("d{d}"), "dependabot[bot]", d, vec![file("Cargo.toml", 1, 1, 1)]));
    }
    let hf = head(&[("Cargo.toml", 1)]);
    for (bots, want) in [(false, 1), (true, 4)] {
        let opts = Options { include_bots: bots, ..Options::default() };
        let ins = Insights::new(&recs, opts, None);
        let changes = ins.hotspots(&hf, Period::All).files[0].changes;
        assert_eq!(ins.file_history("Cargo.toml").len(), want, "bots {bots}");
        assert_eq!(changes, want, "bots {bots}");
    }
}

/// 150 files in three directories over 90 commits by four people, one file renamed: wide
/// enough that a directory card reads more than one chunk of touches.
fn wide_history() -> (Vec<crate::CommitRecord>, Vec<HeadFile>) {
    let who = ["Ann", "Bob", "Cy", "Di"];
    let name = |n: usize| format!("{}/f{n}.rs", ["a", "b", "c"][n % 3]);
    let mut recs = Vec::new();
    for i in 0..90usize {
        let mut touched: Vec<_> = (0..4).map(|k| (i * 7 + k * 13) % 150).collect();
        touched.dedup();
        let files = touched.iter().map(|&n| file(&name(n), 3, 1, 10 + n as u32)).collect();
        recs.push(rec(&format!("c{i}"), who[i % 4], i as i64 * 6, files));
    }
    recs.push(rec("mv", "Ann", 600, vec![renamed("a/f0.rs", "a/moved.rs")]));
    let mut head: Vec<HeadFile> =
        (0..150).map(|n| HeadFile { path: name(n), lines: 10 + n as u32 }).collect();
    head[0].path = "a/moved.rs".into();
    (recs, head)
}

/// Counts the card of every path through `ins` over `c`, against the full reports narrowed to
/// the path: hotspots for churn and rank, coupling for partners, the top-level directory rows
/// of ownership for owners.
fn assert_cards_equal_reports(
    ins: &Insights<'_>,
    c: &crate::Counted<'_>,
    head: &[HeadFile],
    case: &str,
) {
    let hot = ins.hotspots_in(head, c);
    let pairs = ins.coupling_in(head, c, super::PARTNER_MIN_TOGETHER, usize::MAX);
    let own = ins.ownership_in(head, c, 1);
    for h in head {
        let card = ins.path_card(head, c, &h.path);
        let at = hot.files.iter().position(|f| f.path == h.path);
        let row = at.map(|i| &hot.files[i]);
        let tag = format!("{case}: {}", h.path);
        assert_eq!(card.kind, super::PathKind::File, "{tag}");
        assert_eq!(card.changes, row.map_or(0, |r| r.changes), "{tag}: changes");
        assert_eq!(card.lines_changed, row.map_or(0, |r| r.lines_changed), "{tag}: lines");
        assert_eq!(card.rank, at.map_or(0, |i| i + 1), "{tag}: rank");
        assert_eq!(card.of, if at.is_some() { hot.files.len() } else { 0 }, "{tag}: of");
        assert_eq!(card.hot, at.is_some_and(|i| i < hot.top_tenth), "{tag}: hot");
        assert_eq!(card.last_change, row.map(|r| r.last_change), "{tag}: last change");
        assert!(card.authors_90d <= row.map_or(0, |r| r.authors), "{tag}: authors");
        let want: Vec<(String, usize)> = pairs
            .iter()
            .filter(|p| p.a == h.path || p.b == h.path)
            .take(super::CARD_PARTNERS)
            .map(|p| (if p.a == h.path { p.b.clone() } else { p.a.clone() }, p.together))
            .collect();
        let got: Vec<(String, usize)> =
            card.partners.iter().map(|p| (p.path.clone(), p.together)).collect();
        assert_eq!(got, want, "{tag}: partners");
        let in_window: u32 = card.series.iter().sum();
        assert!(in_window as usize <= card.changes, "{tag}: sparkline");
    }
    for d in ["a", "b", "c"] {
        let card = ins.path_card(head, c, d);
        let under: Vec<&crate::Hotspot> =
            hot.files.iter().filter(|f| f.path.starts_with(&format!("{d}/"))).collect();
        let tag = format!("{case}: dir {d}");
        assert_eq!(card.kind, super::PathKind::Dir, "{tag}");
        assert_eq!(card.changes, under.iter().map(|f| f.changes).sum::<usize>(), "{tag}");
        let best = hot.files.iter().position(|f| f.path.starts_with(&format!("{d}/")));
        assert_eq!(card.rank, best.map_or(0, |i| i + 1), "{tag}: rank");
        let row = own.dirs.iter().find(|r| r.path == d);
        let owners: Vec<_> = card.owners.iter().map(|o| (&o.name, o.share.to_bits())).collect();
        let want: Vec<_> = row
            .map(|r| r.owners.iter().take(3).map(|o| (&o.name, o.share.to_bits())).collect())
            .unwrap_or_default();
        assert_eq!(owners, want, "{tag}: owners");
        assert_eq!(card.bus_factor, row.map_or(0, |r| r.bus_factor), "{tag}: bus factor");
        assert!(!card.children.is_empty(), "{tag}: children");
    }
}

/// Oracle: the card for path P is the full report filtered to P, for every file and top-level
/// directory, over the whole history, a year and under a selection.
#[test]
fn the_card_of_a_path_equals_the_full_report_filtered_to_it() {
    use crate::feed::{FixedClock, MemorySource};
    use crate::{CountContext, Scope, ScopeParams};
    let (recs, head) = wide_history();
    let ins = Insights::new(&recs, Options::default(), None);
    let cx = CountContext::new(&MemorySource::new(&recs), &FixedClock(i64::MAX));
    for (case, q, period) in [
        ("all", None, Period::All),
        ("year", None, Period::Year),
        ("author", Some("author:ann@x.io"), Period::All),
        ("path", Some("path:a"), Period::All),
    ] {
        let mut p = ScopeParams::default();
        p.select.q = q.map(str::to_owned);
        let s = Scope::from_params(&p, Options::default(), period).expect("scope");
        assert_cards_equal_reports(&ins, &ins.counted(&s, cx), &head, case);
    }
}

/// The plan that reads a directory's touches is not the answer: any chunk size, and one visit
/// that builds the tip index, give the same card.
#[test]
fn a_directory_card_does_not_depend_on_the_touch_chunk() {
    let (recs, head) = wide_history();
    let ins = Insights::new(&recs, Options::default(), None);
    ins.in_period(Period::All, |c| {
        let want = ins.path_card_chunked(&head, c, "", 0);
        for chunk in [1, 7, 64, 1000] {
            assert_eq!(ins.path_card_chunked(&head, c, "", chunk), want, "chunk {chunk}");
        }
        assert!(want.series.iter().sum::<u32>() > 0);
    });
}

#[test]
fn a_path_outside_the_tree_has_an_empty_card() {
    let (recs, head) = wide_history();
    let ins = Insights::new(&recs, Options::default(), None);
    ins.in_period(Period::All, |c| {
        let card = ins.path_card(&head, c, "a/f0.rs");
        assert_eq!(card.kind, super::PathKind::Missing, "the old name of a renamed file");
        assert_eq!((card.changes, card.rank, card.owners.len()), (0, 0, 0));
        assert_eq!(card.series.len(), super::CARD_BUCKETS);
        let none = ins.path_card(&head, c, "nope/");
        assert_eq!(none.kind, super::PathKind::Missing);
    });
}
