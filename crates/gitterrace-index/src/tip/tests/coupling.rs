//! Coupling over the tip index against every pair counted from the records.

use std::collections::HashMap;

use crate::feed::InsightsSource;
use crate::insights::HeadFile;
use crate::tip::tests::{DAY, EDGE, Hist, commit, disk, f, index};
use crate::{Insights, Options, Period};

/// Coupling over the tip index against pairs counted straight from the records: every pair of
/// head files in one counted commit of 2 to 30 files, at the page's threshold and below it.
/// Guard class: a pair count that drops, doubles or misplaces a commit's pairs (a sort, a
/// prefilter or a dense-row count that diverges from the definition); the page oracle above
/// holds too few shared commits to see that.
#[test]
fn coupling_counts_every_pair_of_a_commit() {
    let h = Hist::default();
    // 90 files, 300 commits: commit n changes the files n*k mod 90 for k in 1..=size, so
    // pairs recur across commits; every seventh commit is a sweep over the 30-file cap.
    let name = |i: usize| format!("src/f{i:02}.rs");
    let mut want: HashMap<(String, String), usize> = HashMap::new();
    for n in 0..300 {
        let size = if n % 7 == 6 { 31 } else { 2 + n % 5 };
        let mut files: Vec<usize> = (1..=size).map(|k| (n * 7 + k * k) % 90).collect();
        // Rare pairs 90+2k and 91+2k change only together, k times: their pair sits exactly
        // on the threshold, so a filter on file changes that is off by one drops it.
        for k in 1..=6 {
            if (k * 40..k * 40 + k).contains(&n) {
                files.extend([90 + 2 * k, 91 + 2 * k]);
            }
        }
        files.sort_unstable();
        files.dedup();
        let who = ["Ann", "Bob", "Cy"][n % 3];
        commit(
            &h,
            n,
            who,
            EDGE + n as i64 * DAY,
            0,
            files.iter().map(|&i| f(&name(i), 1, 0)).collect(),
        );
        if (2..=30).contains(&files.len()) {
            for (x, &a) in files.iter().enumerate() {
                for &b in &files[x + 1..] {
                    // Coupling names a pair in path order.
                    let (p, q) = (name(a), name(b));
                    let key = if p < q { (p, q) } else { (q, p) };
                    *want.entry(key).or_default() += 1;
                }
            }
        }
    }
    let tmp = tempfile::tempdir().expect("tmp");
    let dir = tmp.path().join("gitterrace");
    index(&dir, &h);
    let d = disk(&dir);
    assert!(d.has_tip());
    let files: Vec<HeadFile> = (0..104)
        .map(|i| HeadFile {
            path: name(i),
            lines: 3,
        })
        .collect();
    let ins = Insights::from_source(&d, Options::default(), None);
    for min in 1..=7 {
        let got: HashMap<(String, String), usize> = ins
            .coupling(&files, Period::All, min, usize::MAX)
            .into_iter()
            .map(|c| ((c.a, c.b), c.together))
            .collect();
        let expect: HashMap<(String, String), usize> = want
            .iter()
            .filter(|(_, n)| **n >= min)
            .map(|(k, n)| (k.clone(), *n))
            .collect();
        let mut diff: Vec<_> = got
            .iter()
            .filter(|(k, v)| expect.get(*k) != Some(*v))
            .map(|(k, v)| (k.clone(), Some(*v), expect.get(k).copied()))
            .chain(
                expect
                    .iter()
                    .filter(|(k, _)| !got.contains_key(*k))
                    .map(|(k, v)| (k.clone(), None, Some(*v))),
            )
            .collect();
        diff.sort();
        assert!(
            diff.is_empty(),
            "min_together {min}: (pair, got, want) {diff:?}"
        );
    }
}
