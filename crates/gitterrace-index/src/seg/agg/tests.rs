//! Unit tests for the segment aggregates.

use std::collections::BTreeMap;

use super::Aggregates;
use crate::insights::fixtures::{chain, file, rec};
use crate::insights::{Insights, Options, Period};

fn fixture() -> Vec<crate::CommitRecord> {
    let mut v = chain();
    let mut renamed = file("z.rs", 2, 1, 30);
    renamed.old = Some("x.rs".into());
    let mut m = rec("e", "Cy", 50, vec![file("y.py", 9, 9, 9)]);
    m.merge = true;
    // y.py twice in one commit: still one touch.
    v.push(rec(
        "f",
        "Cy",
        45,
        vec![renamed, file("y.py", 4, 0, 3204), file("y.py", 1, 0, 3205)],
    ));
    v.push(m);
    v
}

/// Path x author summed over authors equals today's `file_churn` for every path.
#[test]
fn path_author_sums_equal_file_churn() {
    let recs = fixture();
    let (ids, agg) = Aggregates::build(&recs);
    let ins = Insights::new(&recs, Options::default(), None);
    let today = ins.file_churn(Period::All);
    let ws = Options::default().count_whitespace;
    let mut sums: BTreeMap<String, (usize, u64, u64)> = BTreeMap::new();
    for ((p, _), v) in &agg.path_author {
        let e = sums
            .entry(ids.paths.list[*p as usize].to_string())
            .or_default();
        e.0 += usize::try_from(v.touches).unwrap_or(0);
        let (a, d) = if ws {
            (v.lines.add, v.lines.del)
        } else {
            (v.lines.code_add, v.lines.code_del)
        };
        e.1 += a;
        e.2 += d;
    }
    sums.retain(|_, v| v.0 > 0);
    assert_eq!(sums.len(), today.len());
    for c in today {
        assert_eq!(
            sums[&c.path],
            (c.commits, c.additions, c.deletions),
            "{}",
            c.path
        );
    }
}

/// Author x day commits equal today's counted commits per author.
#[test]
fn author_day_counts_equal_counted_commits() {
    let recs = fixture();
    let (ids, agg) = Aggregates::build(&recs);
    let ins = Insights::new(&recs, Options::default(), None);
    let mut today: BTreeMap<String, u64> = BTreeMap::new();
    let src = ins.source();
    for c in ins.counted_rows(Period::All) {
        let (name, email) = src.ident(c.ident).unwrap_or_default();
        *today.entry(format!("{name}\u{0}{email}")).or_default() += 1;
    }
    let mut ours: BTreeMap<String, u64> = BTreeMap::new();
    for ((_, who), v) in &agg.author_day {
        *ours
            .entry(ids.identities.list[*who as usize].to_string())
            .or_default() += v.commits;
    }
    assert_eq!(ours, today);
}

#[test]
fn path_author_and_ids_follow_first_use() {
    let (ids, agg) = Aggregates::build(&fixture());
    let names: Vec<&str> = ids.paths.list.iter().map(|s| &**s).collect();
    assert_eq!(names, ["x.rs", "y.py", "z.rs"]);
    let bob = ids.identities.get("Bob\u{0}bob@x.io");
    let y = ids.paths.get("y.py");
    let (Some(bob), Some(y)) = (bob, y) else {
        panic!("ids missing");
    };
    let row = agg.path_author[&(y, bob)];
    assert_eq!((row.touches, row.lines.add, row.lines.del), (2, 3300, 100));
    // The merge counts only as a merge.
    assert!(ids.identities.get("Cy\u{0}cy@x.io").is_some());
    assert_eq!(agg.author_day.values().map(|v| v.commits).sum::<u64>(), 5);
    assert_eq!(agg.author_day.values().map(|v| v.merges).sum::<u64>(), 1);
}

/// Encoded blocks decode to the same rows (the sink is the only writer, so this is the
/// whole byte contract of sections 6 to 10).
#[test]
fn aggregates_round_trip_through_blocks() {
    use crate::seg::bounds::Cursor;
    use crate::seg::format::section;
    let (ids, agg) = Aggregates::build(&fixture());
    let mut blocks: Vec<(u16, u32, Vec<u8>)> = Vec::new();
    let r = agg.encode(ids.paths.list.len(), &mut |s, n, b| {
        blocks.push((s, n, b.to_vec()));
        Ok(())
    });
    assert!(r.is_ok());
    let mut back = Aggregates::default();
    let mut canon = Vec::new();
    for (s, n, b) in &blocks {
        let mut c = Cursor::new(b, "t", 0);
        let r = match *s {
            section::AUTHOR_DAY => super::decode_author_day(&mut c, *n, |d, w, v| {
                back.author_day.insert((d, w), v);
            }),
            section::CANON => {
                canon.push(b.as_slice());
                Ok(())
            }
            section::PATH_AUTHOR => super::decode_path_author(&mut c, *n, |p, w, v| {
                back.path_author.insert((p, w), v);
            }),
            section::PATH_TOUCH => super::decode_life_month(&mut c, *n, |l, m, v| {
                back.life_month.insert((l, m), v);
            }),
            _ => super::decode_path_author(&mut c, *n, |l, w, v| {
                back.life_author.insert((l, w), v);
            }),
        };
        assert!(r.is_ok(), "{r:?}");
    }
    assert_eq!(back.author_day, agg.author_day);
    assert_eq!(back.path_author, agg.path_author);
    assert_eq!(back.life_month, agg.life_month);
    assert_eq!(back.life_author, agg.life_author);
    let lives = crate::seg::canon::Lives::decode(&canon, ids.paths.list.len(), "t");
    assert_eq!(lives.ok(), Some(agg.lives));
    // Truncated block: an error, never a panic.
    let (s, n, b) = &blocks[0];
    let mut c = Cursor::new(&b[..b.len() - 1], "t", 0);
    assert!(
        super::decode_author_day(&mut c, *n, |_, _, _| {}).is_err(),
        "{s}"
    );
}
