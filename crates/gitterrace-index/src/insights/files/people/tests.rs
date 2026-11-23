//! The knowledge risk report against a brute-force recount of the commits.

use std::collections::BTreeMap;

use super::{KnowledgeRisk, RiskQuery};
use crate::insights::fixtures::{DAYS, file, rec};
use crate::insights::{HeadFile, bus_factor};
use crate::{CommitRecord, Insights, Options, Period};

/// Days after the first commit at which the history ends.
const END: i64 = 900;

/// Ann, Cid and Bob stop more than a year before the history ends, Dee does not.
fn history() -> (Vec<CommitRecord>, Vec<HeadFile>) {
    let mut recs = Vec::new();
    let mut add = |who: &str, day: i64, files: &[(&str, u32)]| {
        let id = format!("c{}", recs.len());
        recs.push(rec(&id, who, day, files.iter().map(|(p, n)| file(p, *n, 0, *n)).collect()));
    };
    add("Ann", 0, &[("old/a.rs", 40), ("old/b.rs", 30), ("mix/m.rs", 20), ("top.rs", 5)]);
    add("Cid", 5, &[("old/c.rs", 12), ("mix/m.rs", 15), ("pair/p.rs", 10)]);
    add("Bob", 10, &[("live/l.rs", 50), ("mix/m.rs", 2), ("pair/p.rs", 11)]);
    add("Bob", 300, &[("live/l2.rs", 9), ("old/b.rs", 1)]);
    add("Dee", END, &[("live/l.rs", 3), ("tail/t.rs", 4)]);
    let head = [
        ("old/a.rs", 40),
        ("old/b.rs", 31),
        ("old/c.rs", 12),
        ("mix/m.rs", 37),
        ("pair/p.rs", 21),
        ("live/l.rs", 53),
        ("live/l2.rs", 9),
        ("tail/t.rs", 4),
        ("top.rs", 5),
    ];
    (recs, head.iter().map(|(p, n)| HeadFile { path: (*p).into(), lines: *n }).collect())
}

/// Lines changed per person for each head file, from the records alone.
fn brute(recs: &[CommitRecord], head: &[HeadFile]) -> BTreeMap<String, BTreeMap<String, u64>> {
    let mut out: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for r in recs {
        for f in &r.files {
            if head.iter().any(|h| h.path == *f.path) {
                let who = out.entry(f.path.to_string()).or_default();
                *who.entry(r.author.clone()).or_default() += u64::from(f.additions + f.deletions);
            }
        }
    }
    out
}

fn report(recs: &[CommitRecord], head: &[HeadFile], q: &RiskQuery<'_>) -> Option<KnowledgeRisk> {
    let ins = Insights::new(recs, Options::default(), None);
    ins.in_period(Period::All, |c| ins.knowledge_risk_in(head, c, q))
}

fn all<'a>(path: Option<&'a str>, author: Option<&'a str>) -> RiskQuery<'a> {
    RiskQuery { path, author, limit: 500 }
}

fn last_seen(recs: &[CommitRecord], who: &str) -> i64 {
    recs.iter().filter(|r| r.author == who).map(|r| r.time).max().unwrap_or(i64::MIN)
}

#[test]
fn the_directories_equal_a_brute_force_recount() {
    let (recs, head) = history();
    let by_file = brute(&recs, &head);
    let end = recs.iter().map(|r| r.time).max().unwrap_or_default();
    let idle = |who: &str| last_seen(&recs, who) < end - 365 * DAYS;
    let got = report(&recs, &head, &all(None, None)).expect("report");
    let mut want: BTreeMap<String, (u64, usize, BTreeMap<String, f64>)> = BTreeMap::new();
    for h in &head {
        let who = &by_file[&h.path];
        let total: u64 = who.values().sum();
        let dir = h.path.split_once('/').map_or("", |(d, _)| d).to_owned();
        let e = want.entry(dir).or_default();
        e.0 += u64::from(h.lines);
        e.1 += 1;
        for (p, n) in who {
            *e.2.entry(p.clone()).or_default() += f64::from(h.lines) * *n as f64 / total as f64;
        }
    }
    assert_eq!(got.dirs.len(), want.len());
    let (mut risk_lines, mut risk_dirs) = (0, 0);
    for d in &got.dirs {
        let (lines, files, w) = &want[&d.path];
        let mut owners: Vec<(&String, &f64)> = w.iter().collect();
        owners.sort_by(|a, b| b.1.total_cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let bus = bus_factor(owners.iter().map(|o| *o.1));
        assert_eq!((d.lines, d.files, d.bus_factor), (*lines, *files, bus), "{}", d.path);
        assert_eq!(d.top_owner, *owners[0].0, "{}", d.path);
        assert_eq!(d.owner_active, !idle(&d.top_owner), "{}", d.path);
        assert_eq!(d.at_risk, bus == 1 && idle(&d.top_owner), "{}", d.path);
        assert_eq!(d.last_active, last_seen(&recs, &d.top_owner), "{}", d.path);
        if d.at_risk {
            risk_lines += d.lines;
            risk_dirs += 1;
        }
    }
    assert_eq!((got.at_risk_lines, got.at_risk_dirs), (risk_lines, risk_dirs));
    assert!(risk_dirs > 0, "the history has a directory at risk");
}

#[test]
fn orphans_are_the_files_whose_main_authors_are_all_idle() {
    let (recs, head) = history();
    let by_file = brute(&recs, &head);
    let end = recs.iter().map(|r| r.time).max().unwrap_or_default();
    let idle = |who: &str| last_seen(&recs, who) < end - 365 * DAYS;
    let mut want: Vec<(u64, String)> = Vec::new();
    for h in &head {
        let who = &by_file[&h.path];
        let total: u64 = who.values().sum();
        let top = who.values().max().copied().unwrap_or_default();
        let main = who.iter().filter(|(_, n)| **n == top || **n as f64 / total as f64 >= 0.1);
        if main.into_iter().all(|(p, _)| idle(p)) {
            want.push((u64::from(h.lines), h.path.clone()));
        }
    }
    want.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let got = report(&recs, &head, &all(None, None)).expect("report");
    let listed: Vec<(u64, String)> =
        got.orphans.iter().map(|o| (o.lines, o.path.clone())).collect();
    assert_eq!(listed, want);
    assert_eq!(got.orphan_files, want.len());
    assert_eq!(got.orphan_lines, want.iter().map(|w| w.0).sum::<u64>());
    // The bounded heap keeps the largest rows and still counts every orphan.
    let cut = report(&recs, &head, &RiskQuery { limit: 2, ..all(None, None) }).expect("report");
    assert_eq!(cut.orphans.len(), 2);
    assert_eq!(cut.orphans[0].path, want[0].1);
    assert_eq!(cut.orphan_files, want.len());
    let newest = recs.iter().filter(|r| r.files.iter().any(|f| *f.path == *want[0].1));
    assert_eq!(cut.orphans[0].last_change, newest.map(|r| r.time).max().unwrap_or_default());
}

#[test]
fn drilling_lists_the_children_of_the_path() {
    let (recs, head) = history();
    let got = report(&recs, &head, &all(Some("old"), None)).expect("report");
    let paths: Vec<&str> = got.dirs.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["old"], "files directly in the path group under the path");
    assert_eq!(got.dirs[0].files, 3);
}

#[test]
fn a_person_holds_the_files_they_own_most() {
    let (recs, head) = history();
    let by_file = brute(&recs, &head);
    let got = report(&recs, &head, &all(None, Some("Bob"))).expect("report");
    let p = got.person.expect("person");
    let owned: Vec<&HeadFile> = head
        .iter()
        .filter(|h| {
            let w = &by_file[&h.path];
            let top = w.values().max().copied().unwrap_or_default();
            w.get("Bob") == Some(&top)
        })
        .collect();
    assert_eq!(p.files, owned.len());
    assert_eq!(p.lines, owned.iter().map(|h| u64::from(h.lines)).sum::<u64>());
    let repo: u64 = head.iter().map(|h| u64::from(h.lines)).sum();
    assert!((p.share_of_repo - p.lines as f64 / repo as f64).abs() < 1e-9);
    assert_eq!(p.last_active, last_seen(&recs, "Bob"));
    assert!(p.top_files.windows(2).all(|w| w[0].lines >= w[1].lines));
    assert!(got.dirs.iter().all(|d| d.top_owner == "Bob"));
    assert!(report(&recs, &head, &all(None, Some("Nobody"))).is_none());
}
