//! Histories for rename-aware counting: a rename chain with every case the canonical name
//! must handle, and a merge made by a person with no other commit that day.

use std::path::Path;

use crate::Fixture;

/// Body of `lines` numbered lines tagged `tag`, long enough for rename detection to pair a
/// moved file with its old name.
fn body(tag: &str, lines: usize) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    for i in 0..lines {
        writeln!(s, "{tag} line {i}").expect("write string");
    }
    s
}

/// Commits as `name` from here on.
fn by(f: Fixture, name: &str) -> Fixture {
    f.git(&["config", "user.name", name]);
    f.git(&[
        "config",
        "user.email",
        &format!("{}@x.io", name.to_lowercase()),
    ]);
    f
}

/// The rename chain history. Built under `dir/repo`, oldest first:
///
/// 1. Ann adds `a.rs`, `x.rs`, `Foo.rs`, `r.rs` and `keep.rs`.
/// 2. Bob edits `a.rs` and `x.rs`.
/// 3. Ann renames `a.rs` to `b.rs`. 4. Cy edits `b.rs`.
/// 5. Bob renames `b.rs` to `c.rs` and edits `keep.rs` in the same commit.
/// 6. Cy renames `x.rs` to `y.rs`. 7. Ann edits `y.rs`.
/// 8. Bob deletes `y.rs` (a dead chain) and edits `c.rs`, at 23:30 -0800 on 2024-01-31: the
///    last local day of January, already February in UTC.
/// 9. Ann copies `c.rs` to `d.rs` (a copy, not followed) and edits `c.rs`.
/// 10. Cy deletes `r.rs`. 11. Ann adds a new `r.rs` (a new life under a reused name).
/// 12. Bob renames `Foo.rs` to `foo.rs` (case only).
/// 13. Cy edits `foo.rs`, `r.rs` and `c.rs`.
/// 14. Ann adds `x2.rs`. A branch renames it to `y2.rs`, main renames it to `z2.rs`, Cy merges
///     keeping both, Ann edits both (a fork of one life).
pub struct RenameChain {
    pub repo: std::path::PathBuf,
    /// The commit made at 23:30 -0800 on the last day of a month.
    pub month_end: String,
    pub tip: String,
}

impl RenameChain {
    #[must_use]
    pub fn build(dir: &Path) -> Self {
        let repo = dir.join("repo");
        let f = by(Fixture::init(&repo), "Ann")
            .at("2024-01-02T10:00:00Z")
            .write("a.rs", &body("a", 20))
            .write("x.rs", &body("x", 20))
            .write("Foo.rs", &body("foo", 20))
            .write("r.rs", &body("r", 20))
            .write("keep.rs", &body("keep", 5))
            .commit("feat: first files");
        let f = by(f, "Bob")
            .at("2024-01-05T10:00:00Z")
            .write("a.rs", &(body("a", 20) + "a more\n"))
            .write("x.rs", &(body("x", 20) + "x more\n"))
            .commit("fix: grow a and x");
        let f = by(f, "Ann").at("2024-01-10T10:00:00Z");
        f.git(&["mv", "a.rs", "b.rs"]);
        let f = f.commit("refactor: a to b");
        let f = by(f, "Cy")
            .at("2024-01-15T10:00:00Z")
            .write("b.rs", &(body("a", 20) + "a more\nb edit\n"))
            .commit("fix: edit b");
        let f = by(f, "Bob").at("2024-01-20T10:00:00Z");
        f.git(&["mv", "b.rs", "c.rs"]);
        let f = f
            .write("keep.rs", &(body("keep", 5) + "with c\n"))
            .commit("refactor: b to c");
        let f = by(f, "Cy").at("2024-01-25T10:00:00Z");
        f.git(&["mv", "x.rs", "y.rs"]);
        let f = f.commit("refactor: x to y");
        let f = by(f, "Ann")
            .at("2024-01-28T10:00:00Z")
            .write("y.rs", &(body("x", 20) + "x more\ny edit\n"))
            .commit("fix: edit y");
        let f = by(f, "Bob").at("2024-01-31T23:30:00-08:00");
        f.git(&["rm", "-q", "y.rs"]);
        let f = f
            .write("c.rs", &(body("a", 20) + "a more\nb edit\nmonth end\n"))
            .commit("fix: drop y at the month end");
        let month_end = f.git(&["rev-parse", "HEAD"]);
        let c = body("a", 20) + "a more\nb edit\nmonth end\n";
        let f = by(f, "Ann")
            .at("2024-02-03T10:00:00Z")
            .write("d.rs", &c)
            .write("c.rs", &(c.clone() + "after copy\n"))
            .commit("feat: copy c to d");
        let f = by(f, "Cy").at("2024-02-05T10:00:00Z");
        f.git(&["rm", "-q", "r.rs"]);
        let f = f.commit("chore: drop r");
        let f = by(f, "Ann")
            .at("2024-02-08T10:00:00Z")
            .write("r.rs", "a new r\n")
            .commit("feat: a new r");
        let f = by(f, "Bob").at("2024-02-10T10:00:00Z");
        f.git(&["mv", "Foo.rs", "foo.rs"]);
        let f = f.commit("refactor: lower case foo");
        let f = by(f, "Cy")
            .at("2024-02-12T10:00:00Z")
            .write("foo.rs", &(body("foo", 20) + "foo edit\n"))
            .write("r.rs", "a new r\nedited\n")
            .write("c.rs", &(c + "after copy\ntip\n"))
            .commit("fix: edit foo, r and c");
        // A fork: side renames x2 to y2, main renames it to z2, the merge keeps both.
        let f = by(f, "Ann")
            .at("2024-02-14T10:00:00Z")
            .write("x2.rs", &body("x2", 20))
            .commit("feat: add x2")
            .branch("fork");
        let f = f.at("2024-02-15T10:00:00Z");
        f.git(&["mv", "x2.rs", "y2.rs"]);
        let f = f.commit("refactor: x2 to y2").checkout("main");
        let f = by(f, "Bob").at("2024-02-16T10:00:00Z");
        f.git(&["mv", "x2.rs", "z2.rs"]);
        let f = f.commit("refactor: x2 to z2");
        let f = by(f, "Cy").at("2024-02-17T10:00:00Z");
        drop(crate::try_git(
            f.path(),
            &["merge", "-q", "--no-ff", "fork"],
        ));
        f.git(&["checkout", "fork", "--", "y2.rs"]);
        f.git(&["rm", "-q", "--cached", "--ignore-unmatch", "x2.rs"]);
        let f = f
            .write("z2.rs", &body("x2", 20))
            .commit("merge fork, keep y2 and z2");
        f.git(&["branch", "-q", "-D", "fork"]);
        let f = by(f, "Ann")
            .at("2024-02-18T10:00:00Z")
            .write("y2.rs", &(body("x2", 20) + "y2 edit\n"))
            .write("z2.rs", &(body("x2", 20) + "z2 edit\n"))
            .commit("fix: edit y2 and z2");
        let tip = f.git(&["rev-parse", "HEAD"]);
        Self {
            repo,
            month_end,
            tip,
        }
    }
}

/// A merge commit by `Mia`, who makes no other commit on that day, over a side branch by Ann
/// and a main branch by Bob. Built under `dir/repo`.
pub struct MergeWithAuthor {
    pub repo: std::path::PathBuf,
    pub merge: String,
}

impl MergeWithAuthor {
    #[must_use]
    pub fn build(dir: &Path) -> Self {
        let repo = dir.join("repo");
        let f = by(Fixture::init(&repo), "Ann")
            .at("2024-03-01T10:00:00Z")
            .write("a.rs", &body("a", 5))
            .commit("feat: first")
            .branch("side")
            .at("2024-03-02T10:00:00Z")
            .write("s.rs", &body("s", 5))
            .commit("feat: side work")
            .checkout("main");
        let f = by(f, "Bob")
            .at("2024-03-03T10:00:00Z")
            .write("a.rs", &(body("a", 5) + "main\n"))
            .commit("fix: main work");
        let f = by(f, "Mia").at("2024-03-04T10:00:00Z");
        f.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
        let merge = f.git(&["rev-parse", "HEAD"]);
        let f = by(f, "Bob")
            .at("2024-03-05T10:00:00Z")
            .write("a.rs", &(body("a", 5) + "main\nafter\n"))
            .commit("fix: after the merge");
        drop(f);
        Self { repo, merge }
    }
}
