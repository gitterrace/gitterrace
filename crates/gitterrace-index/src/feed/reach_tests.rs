use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use crate::feed::disk_testing::write_cache;
use crate::feed::{CommitRange, DiskSource, InsightsSource, MemorySource, Posting};
use crate::feed::{RevView, rev_view};
use crate::seg::read::Store;
use crate::{CommitRecord, CommitSource, FileStat};

/// Records by id. Counts reads, so a test sees how much git a view cost.
struct Git {
    recs: HashMap<String, CommitRecord>,
    reads: std::sync::atomic::AtomicUsize,
}

impl CommitSource for Git {
    fn rev_list(&self, rev: &str) -> io::Result<Vec<String>> {
        let mut out: Vec<&CommitRecord> = Vec::new();
        let mut todo = vec![rev.to_owned()];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = todo.pop() {
            if seen.insert(id.clone())
                && let Some(r) = self.recs.get(&id)
            {
                todo.extend(r.parents.iter().cloned());
                out.push(r);
            }
        }
        // Times rise along every edge in the fixture, so time order is canonical order.
        out.sort_by_key(|r| (r.time, r.id.clone()));
        Ok(out.into_iter().map(|r| r.id.clone()).collect())
    }

    fn record(&self, id: &str) -> io::Result<CommitRecord> {
        self.reads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.recs
            .get(id)
            .cloned()
            .ok_or_else(|| io::Error::other("absent"))
    }
}

fn rec(
    id: &str,
    t: i64,
    parents: &[&str],
    who: &str,
    files: &[(&str, Option<&str>)],
) -> CommitRecord {
    CommitRecord {
        id: id.into(),
        author: who.into(),
        email: format!("{who}@x.io"),
        time: 1_700_000_000 + t * 3600,
        offset: 3600,
        merge: parents.len() > 1,
        parents: parents.iter().map(|p| (*p).to_owned()).collect(),
        kind: (t % 2 == 0).then(|| if t > 20 { "perf" } else { "feat" }.into()),
        scope: None,
        files: files
            .iter()
            .map(|(p, old)| FileStat {
                path: (*p).into(),
                old: old.map(Into::into),
                additions: u32::try_from(t).unwrap_or(0) + 1,
                deletions: 1,
                lines: 9,
                ..FileStat::default()
            })
            .collect(),
    }
}

fn oid(n: u32) -> String {
    format!("{n:040x}")
}

/// main m0..m9, side s1..s2 off m3 merged at m6, then unindexed s3, s4, s5 on s2 (s5 new
/// author, new path, new label, a rename).
fn history() -> (Vec<CommitRecord>, Vec<CommitRecord>) {
    let m = |i: u32| oid(i);
    let s = |i: u32| oid(100 + i);
    let mut set = Vec::new();
    for i in 0..10u32 {
        let t = i64::from(i) * 2;
        let parents: Vec<String> = match i {
            0 => vec![],
            6 => vec![m(5), s(2)],
            _ => vec![m(i - 1)],
        };
        let p: Vec<&str> = parents.iter().map(String::as_str).collect();
        set.push(rec(
            &m(i),
            t,
            &p,
            ["ann", "bob"][i as usize % 2],
            &[("src/a.rs", None)],
        ));
        if i == 3 {
            set.push(rec(&s(1), t + 1, &[&m(3)], "cy", &[("src/b.rs", None)]));
            set.push(rec(
                &s(2),
                t + 1,
                &[&s(1)],
                "cy",
                &[("doc/b.md", Some("src/b.rs"))],
            ));
        }
    }
    set.sort_by_key(|r| (r.time, r.id.clone()));
    let fresh = vec![
        rec(&s(3), 30, &[&s(2)], "cy", &[("doc/b.md", None)]),
        rec(&s(4), 31, &[&s(3)], "ann", &[("src/a.rs", None)]),
        rec(
            &s(5),
            32,
            &[&s(4)],
            "dee",
            &[("new/c.rs", Some("doc/b.md"))],
        ),
    ];
    (set, fresh)
}

/// Everything a page can see, in strings, so two sources with different ids compare.
fn view(s: &dyn InsightsSource) -> Vec<String> {
    let mut out = vec![format!("len {}", s.len())];
    for c in s.commits_in(CommitRange::All) {
        let who = s.ident(c.ident).unwrap_or_default();
        let kind = c.kind.and_then(|k| s.label(k));
        out.push(format!(
            "{:?} {:?} {who:?} {kind:?} {} {} {:?} {:?}",
            c.rank,
            s.commit_id(c.rank),
            c.time,
            c.merge,
            c.sums,
            s.parent_ids(c.rank)
        ));
    }
    let ranks = (0..u32::try_from(s.len()).unwrap_or(0))
        .map(crate::feed::Rank)
        .collect();
    for (r, files) in s.rows_for(ranks) {
        for f in files {
            out.push(format!(
                "row {r:?} {:?} {:?} {}",
                s.path(f.path),
                f.old.and_then(|o| s.path(o)),
                f.add
            ));
        }
    }
    for p in ["src/a.rs", "src/b.rs", "doc/b.md", "new/c.rs"] {
        let hits: Vec<_> = s
            .find_path(p)
            .map(|id| s.postings(Posting::Path(id)).collect())
            .unwrap_or_default();
        out.push(format!("path {p} {hits:?}"));
    }
    for a in s.agg_author_day(i64::MIN..i64::MAX) {
        out.push(format!(
            "day {} {:?} {} {:?}",
            a.day,
            s.ident(a.ident),
            a.commits,
            a.sums
        ));
    }
    let mut pa: Vec<String> = s
        .agg_path_author("")
        .map(|a| {
            format!(
                "pa {:?} {:?} {}",
                s.path(a.path),
                s.ident(a.ident),
                a.touches
            )
        })
        .collect();
    pa.sort();
    out.extend(pa);
    for e in s.rename_edges() {
        out.push(format!(
            "edge {:?} {:?} {:?}",
            e.rank,
            s.path(e.old),
            s.path(e.new)
        ));
    }
    out
}

fn disk(dir: &std::path::Path) -> DiskSource {
    let store = Store::open(dir).ok().flatten().expect("store");
    DiskSource::open(store, &mut |p| Ok(p.to_vec())).expect("open")
}

#[test]
fn a_rev_view_equals_a_fresh_load_of_that_rev() {
    let (set, fresh) = history();
    let dir = tempfile::tempdir().expect("tmp");
    write_cache(dir.path(), &set, &[7]).expect("write");
    let git = Git {
        recs: set
            .iter()
            .chain(&fresh)
            .map(|r| (r.id.clone(), r.clone()))
            .collect(),
        reads: 0.into(),
    };
    for (tip, reads) in [(oid(9), 0), (oid(4), 0), (oid(102), 0), (oid(105), 3)] {
        git.reads.store(0, std::sync::atomic::Ordering::Relaxed);
        let want: Vec<CommitRecord> = git
            .rev_list(&tip)
            .expect("list")
            .iter()
            .map(|id| git.recs[id].clone())
            .collect();
        let mem = MemorySource::shared(Arc::new(want));
        let got = match rev_view(disk(dir.path()), &git, &tip, &tip, 2000).expect("view") {
            RevView::Ready(v) => v,
            RevView::NeedsReindex { .. } => panic!("{tip}: under the cap"),
        };
        assert_eq!(view(&*got), view(&mem), "tip {tip}");
        assert_eq!(got.fresh_len(), reads, "tip {tip}");
        assert_eq!(git.reads.load(std::sync::atomic::Ordering::Relaxed), reads);
    }
}

#[test]
fn past_the_cap_asks_for_a_reindex_and_stops_reading() {
    let (set, fresh) = history();
    let dir = tempfile::tempdir().expect("tmp");
    write_cache(dir.path(), &set, &[]).expect("write");
    let git = Git {
        recs: set
            .iter()
            .chain(&fresh)
            .map(|r| (r.id.clone(), r.clone()))
            .collect(),
        reads: 0.into(),
    };
    let tip = oid(105);
    let v = rev_view(disk(dir.path()), &git, &tip, &tip, 2).expect("view");
    assert!(matches!(v, RevView::NeedsReindex { read: 2 }));
    assert_eq!(git.reads.load(std::sync::atomic::Ordering::Relaxed), 2);
}
