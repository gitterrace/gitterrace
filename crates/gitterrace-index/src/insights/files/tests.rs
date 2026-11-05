//! Unit tests for file history, hotspots and ownership.

use super::super::HeadFile;
use crate::insights::fixtures::{file, rec};
use crate::{FileStat, Insights, Options, Period};

fn head(files: &[(&str, u32)]) -> Vec<HeadFile> {
    files
        .iter()
        .map(|(p, n)| HeadFile {
            path: (*p).into(),
            lines: *n,
        })
        .collect()
}

fn renamed(old: &str, new: &str) -> FileStat {
    FileStat {
        old: Some(old.into()),
        ..file(new, 0, 0, 10)
    }
}

fn history() -> Vec<crate::CommitRecord> {
    vec![
        rec(
            "a",
            "Ann",
            0,
            vec![file("old/x.rs", 10, 0, 10), file("keep.rs", 4, 0, 4)],
        ),
        rec("b", "Bob", 400, vec![file("old/x.rs", 2, 1, 11)]),
        rec(
            "c",
            "Ann",
            401,
            vec![renamed("old/x.rs", "new/x.rs"), file("keep.rs", 1, 1, 4)],
        ),
        rec(
            "d",
            "Ann",
            402,
            vec![file("new/x.rs", 3, 0, 14), file("keep.rs", 1, 0, 5)],
        ),
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
        recs.push(rec(
            &format!("b{n}"),
            "Bob",
            900,
            vec![file(&path, 0, 5, 5)],
        ));
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
    let x = rep
        .files
        .iter()
        .find(|f| f.path == "new/x.rs")
        .map(|f| (f.changes, f.authors));
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
    let nx = o
        .dirs
        .iter()
        .find(|d| d.path == "new")
        .map(|d| d.owners[0].name.clone());
    assert_eq!(nx.as_deref(), Some("Ann"));
    assert_eq!(o.overall.bus_factor, 1);
    let root = o
        .dirs
        .iter()
        .find(|d| d.path.is_empty())
        .map(|d| d.bus_factor);
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
    assert_eq!(
        (c[0].together, c[0].a.as_str(), c[0].b.as_str()),
        (3, "keep.rs", "new/x.rs")
    );
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
    let with = Insights::new(
        &recs,
        Options {
            include_bots: true,
            ..Options::default()
        },
        None,
    );
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
    let mut names: Vec<String> = (0..super::PER_PATH_MAX + 5)
        .map(|n| format!("pad/{n}.rs"))
        .collect();
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
            l.iter()
                .map(|(t, c)| (t.commit, t.row, t.path, t.lines, c.rank))
                .collect::<Vec<_>>()
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
    fn find_commit(&self, id: &str) -> Option<crate::feed::Rank> {
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
        self.lookups
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.postings(k)
    }
    fn rename_edges(&self) -> crate::feed::Stream<'_, crate::feed::RenameEdge> {
        self.inner.rename_edges()
    }
    fn path(&self, id: crate::feed::PathId) -> Option<std::sync::Arc<str>> {
        self.inner.path(id)
    }
    fn find_path(&self, p: &str) -> Option<crate::feed::PathId> {
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
    fn find_label(&self, n: &str) -> Option<crate::feed::LabelId> {
        self.inner.find_label(n)
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
            rec(
                &format!("c{i}"),
                "Ann",
                i64::from(i),
                vec![file(&format!("f{i}.rs"), 1, 0, 1)],
            )
        })
        .collect();
    let hf: Vec<HeadFile> = (0..n)
        .map(|i| HeadFile {
            path: format!("f{i}.rs"),
            lines: 1,
        })
        .collect();
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
    recs.push(rec(
        "e",
        "dependabot[bot]",
        403,
        vec![file("keep.rs", 9, 2, 5)],
    ));
    // Whitespace-only lines: raw and code counts differ.
    recs.push(rec(
        "f",
        "Bob",
        404,
        vec![FileStat {
            code: Some(crate::CodeStat {
                additions: 1,
                deletions: 0,
                rewritten: 0,
            }),
            ..file("keep.rs", 6, 4, 5)
        }],
    ));
    let hf = head(&[("new/x.rs", 14), ("keep.rs", 5), ("gone.rs", 3)]);
    for (bots, ws) in [(false, false), (true, false), (false, true), (true, true)] {
        let opts = Options {
            include_bots: bots,
            count_whitespace: ws,
            ..Options::default()
        };
        let ins = Insights::new(&recs, opts, None);
        let shown: Vec<&HeadFile> = hf.iter().collect();
        let fast = ins.owner_lines(&shown, &(i64::MIN..=i64::MAX), |_| true, true);
        let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
        let slow: Vec<std::collections::BTreeMap<usize, u64>> = ins
            .touches_in(
                &names,
                &(i64::MIN..=i64::MAX),
                &|_: &crate::feed::CommitRow| true,
            )
            .into_iter()
            .map(|t| {
                let mut m = std::collections::BTreeMap::new();
                for (t, c) in t {
                    *m.entry(ins.person_of(c.ident)).or_default() += u64::from(t.lines);
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
    let fast = ins.owner_lines(&shown, &(i64::MIN..=i64::MAX), |_| true, true);
    let names: Vec<&str> = shown.iter().map(|f| &*f.path).collect();
    let slow: Vec<std::collections::BTreeMap<usize, u64>> = ins
        .touches_in(
            &names,
            &(i64::MIN..=i64::MAX),
            &|_: &crate::feed::CommitRow| true,
        )
        .into_iter()
        .map(|t| {
            let mut m = std::collections::BTreeMap::new();
            for (t, c) in t {
                *m.entry(ins.person_of(c.ident)).or_default() += u64::from(t.lines);
            }
            m
        })
        .collect();
    assert_eq!(fast, slow);
    let ann = ins.person_of(crate::feed::IdentId(0));
    assert!(
        !fast[0].contains_key(&ann),
        "the new a.rs has only Bob's lines"
    );
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
        ..rec(
            id,
            who,
            0,
            vec![file("a.rs", 1, 0, 9), file("b.rs", 1, 0, 9)],
        )
    };
    let recs = vec![
        at("late", "Lee", 3 * DAYS + 23 * 3600 + 1800, -7200),
        at("early", "Eve", 4 * DAYS + 3600, 50_400),
        rec(
            "now",
            "Ann",
            10,
            vec![file("a.rs", 1, 0, 9), file("b.rs", 1, 0, 9)],
        ),
    ];
    let ins = Insights::new(&recs, Options::default(), None);
    let hf = head(&[("a.rs", 9), ("b.rs", 9)]);
    let hot = ins.hotspots(&hf, Period::Week);
    assert_eq!(hot.total_changes, 4, "early and now on two files");
    assert_eq!(hot.files[0].authors, 2);
    let own = ins.ownership(&hf, Period::Week, 1);
    let names: Vec<&str> = own.overall.owners.iter().map(|o| o.name.as_str()).collect();
    assert!(
        names.contains(&"Eve") && !names.contains(&"Lee"),
        "{names:?}"
    );
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
        let mut p = ScopeParams {
            period: Some("1w".to_owned()),
            ..ScopeParams::default()
        };
        p.select.path = path.map(str::to_owned);
        Scope::from_params(&p, Options::default(), Period::All).expect("scope")
    };
    let s = scope(None);
    let c = ins.counted(&s, cx);
    assert_eq!(ins.hotspots_in(&hf, &c), ins.hotspots(&hf, Period::Week));
    assert_eq!(
        ins.ownership_in(&hf, &c, 1),
        ins.ownership(&hf, Period::Week, 1)
    );
    assert_eq!(
        ins.coupling_in(&hf, &c, 1, 9),
        ins.coupling(&hf, Period::Week, 1, 9)
    );
    let s = scope(Some("new"));
    let c = ins.counted(&s, cx);
    let hot = ins.hotspots_in(&hf, &c);
    assert_eq!(hot.files.len(), 1);
    assert_eq!(hot.files[0].path, "new/x.rs");
    assert!(ins.coupling_in(&hf, &c, 1, 9).is_empty());
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
            recs.push(rec(
                &format!("e{n}"),
                extra,
                2 + n,
                vec![file("other.rs", 1, 0, 1)],
            ));
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
    p.select.author = Some("ann@x.io".to_owned());
    let s = Scope::from_params(&p, Options::default(), Period::All).expect("scope");
    let years = |r: crate::AgeReport| r.buckets.iter().map(|b| b.year).collect::<Vec<_>>();
    assert_eq!(
        years(ins.code_age_in(&hf, &ins.counted(&s, cx))),
        [2024],
        "Ann's own, no merge"
    );
    assert_eq!(
        years(ins.code_age(&hf)),
        [2025],
        "Bob's change counts without a selection"
    );
}
