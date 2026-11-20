//! Record fixtures for the insights tests.

use crate::{CommitRecord, FileStat};

// 2024-01-01 is a Monday.
pub const MON: i64 = 1_704_067_200;
pub const DAYS: i64 = 86_400;

pub fn file(path: &str, add: u32, del: u32, lines: u32) -> FileStat {
    FileStat { path: path.into(), additions: add, deletions: del, lines, ..FileStat::default() }
}

/// Four commits in a line: a, b, c, d.
pub fn chain() -> Vec<CommitRecord> {
    let mut v = vec![
        rec("a", "Ann", 0, vec![file("x.rs", 1, 0, 1)]),
        rec("b", "Ann", 10, vec![file("x.rs", 30, 0, 31)]),
        rec("c", "Bob", 30, vec![file("y.py", 300, 100, 200), file("x.rs", 0, 3, 28)]),
        rec("d", "Bob", 40, vec![file("y.py", 3000, 0, 3200)]),
    ];
    for i in 1..v.len() {
        v[i].parents = vec![v[i - 1].id.clone()];
    }
    v
}

pub fn rec(id: &str, who: &str, day: i64, files: Vec<FileStat>) -> CommitRecord {
    CommitRecord {
        id: id.into(),
        author: who.into(),
        email: format!("{}@x.io", who.to_lowercase()),
        time: MON + day * DAYS + 3600 * 12,
        files,
        ..CommitRecord::default()
    }
}
