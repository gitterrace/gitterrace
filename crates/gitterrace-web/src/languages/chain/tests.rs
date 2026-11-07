use std::sync::Arc;

use gitterrace_index::feed::{InsightsSource, MemorySource};
use gitterrace_index::{CommitRecord, FileStat, Options, PathFilter};

use super::{Fold, Skeleton, Tally, walk, wanted};
use crate::languages::{Held, View};

/// A linear history of `n` commits over a handful of languages, with deletions, a lock file
/// the filter skips and more languages than the page shows, so `Other` is folded.
fn history(n: usize) -> Vec<CommitRecord> {
    const EXT: [&str; 11] = [
        "rs", "py", "go", "c", "js", "ts", "rb", "md", "sh", "toml", "lock",
    ];
    (0..n)
        .map(|i| {
            let ext = EXT[(i * 7) % EXT.len()];
            let add = u32::try_from((i * 31) % 97 + 1).unwrap_or(1);
            let del = u32::try_from((i * 13) % 23).unwrap_or(0).min(add / 2);
            CommitRecord {
                id: format!("{i:040x}"),
                author: "a".to_owned(),
                email: "a@x.io".to_owned(),
                time: 1_600_000_000 + i64::try_from(i).unwrap_or(0) * 3600,
                parents: if i == 0 {
                    vec![]
                } else {
                    vec![format!("{:040x}", i - 1)]
                },
                files: vec![FileStat {
                    path: Arc::from(format!("src/f{}.{ext}", i % 5).as_str()),
                    additions: add,
                    deletions: del,
                    ..FileStat::default()
                }],
                ..CommitRecord::default()
            }
        })
        .collect()
}

/// Lines after every commit, ids and times, the way the page computed them before.
struct Reference {
    ids: Vec<String>,
    times: Vec<i64>,
    lines: Vec<Vec<i64>>,
    adds: Vec<u64>,
    dels: Vec<u64>,
}

fn reference(src: &dyn InsightsSource, filter: &PathFilter) -> Reference {
    let newest = src
        .len()
        .checked_sub(1)
        .map(|i| gitterrace_index::feed::Rank(u32::try_from(i).unwrap_or(0)));
    let mut chain: Vec<_> = walk(src, newest).collect();
    chain.reverse();
    let mut t = Tally::new(src, filter);
    let mut raw = Vec::new();
    let (mut adds, mut dels) = (Vec::new(), Vec::new());
    for (_, files) in src.rows_for(chain.clone()) {
        let (a, d) = (t.added, t.removed);
        t.apply(&files);
        adds.push(t.added - a);
        dels.push(t.removed - d);
        raw.push(t.net.clone());
    }
    let fold = Fold::new(&t.names, &t.net);
    let lines = raw
        .iter()
        .map(|r| {
            let mut r = r.clone();
            r.resize(t.names.len(), 0);
            fold.apply(&r)
        })
        .collect();
    Reference {
        ids: chain
            .iter()
            .map(|r| src.commit_id(*r).map(|s| s.to_string()).unwrap_or_default())
            .collect(),
        times: chain
            .iter()
            .map(|r| src.commit(*r).map_or(0, |c| c.time))
            .collect(),
        lines,
        adds,
        dels,
    }
}

fn filter() -> PathFilter {
    PathFilter::new(&Options::default(), None)
}

/// Every ranged view reads the same lines, ids, times and range sums as the full chain:
/// a range inside one mark gap, across gaps, at both ends and the whole history.
#[test]
fn views_match_the_full_chain() {
    let recs = history(1500);
    let src = MemorySource::new(&recs);
    let f = filter();
    let want = reference(&src, &f);
    let sk = Arc::new(Skeleton::build(&src, &f));
    let n = sk.len();
    assert_eq!(n, 1500);
    assert!(sk.gap < n, "the fixture spans several mark gaps");
    let ranges = [
        None,
        Some((70, 90)),
        Some((10, 1200)),
        Some((0, 5)),
        Some((n - 3, n - 1)),
        Some((0, n - 1)),
        Some((63, 64)),
    ];
    for range in ranges {
        let view = View {
            time: false,
            share: false,
            range,
            picked: range.is_some(),
            period: gitterrace_index::Period::All,
            dflt: gitterrace_index::Period::All,
            held: Held::Commits,
        };
        let ch = sk.view(&src, &f, view);
        for i in wanted(n, range) {
            assert_eq!(ch.lines(i), want.lines[i], "lines at {i} for {range:?}");
            assert_eq!(ch.id(i), want.ids[i], "id at {i} for {range:?}");
            assert_eq!(ch.time(i), want.times[i], "time at {i} for {range:?}");
        }
        let (a, b) = range.unwrap_or((0, n - 1));
        assert_eq!(ch.added, want.adds[a..=b].iter().sum::<u64>(), "{range:?}");
        assert_eq!(
            ch.removed,
            want.dels[a..=b].iter().sum::<u64>(),
            "{range:?}"
        );
    }
}

/// What a skeleton holds is flat in history length: the marks are capped and the kept
/// positions follow the page, so 100 times the history may not grow it more than a little.
#[test]
fn skeleton_size_is_flat_in_history_length() {
    let f = filter();
    let small = history(2_000);
    let large = history(200_000);
    let a = Skeleton::build(&MemorySource::new(&small), &f).bytes();
    let b = Skeleton::build(&MemorySource::new(&large), &f).bytes();
    assert!(b < 512 * 1024, "skeleton holds {b} bytes at 200k commits");
    assert!(b < a * 4, "skeleton grew from {a} to {b} bytes");
}

/// A tree with signatures, prebuilt binaries and files without an extension next to real
/// languages: the legend names only languages, and every line still counts, under `Other`.
#[test]
fn formats_count_under_other_and_leave_the_legend() {
    let names: Vec<String> = [
        "Rust",
        ".asc",
        "Python",
        ".prebuilt",
        "(no extension)",
        ".png",
    ]
    .map(str::to_owned)
    .to_vec();
    let last = [500, 40, 300, 70, 20, 9];
    let f = Fold::new(&names, &last);
    assert_eq!(f.langs, ["Rust", "Python", "(no extension)", "Other"]);
    let row = f.apply(&last);
    assert_eq!(row, [500, 300, 20, 40 + 70 + 9]);
    assert_eq!(
        row.iter().sum::<i64>(),
        last.iter().sum::<i64>(),
        "no line is lost"
    );
    // Languages alone fold nothing.
    let f = Fold::new(&names[..1], &last[..1]);
    assert_eq!(f.langs, ["Rust"]);
}
