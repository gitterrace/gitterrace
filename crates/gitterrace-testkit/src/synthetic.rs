use std::fmt::Write as _;
use std::path::Path;

use crate::{git, git_stdin};

/// A seeded xorshift64*: the same seed gives the same history on every machine.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            seed
        })
    }

    pub const fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A value in `0..n`, `0` when `n` is `0`.
    pub const fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

/// The shape of a generated history.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub commits: usize,
    /// Distinct paths the commits draw from: a wide fan-out exercises per-path tables.
    pub paths: usize,
    /// Distinct identities, every tenth one a bot.
    pub authors: usize,
    pub files_per_commit: usize,
    pub seed: u64,
}

impl Shape {
    /// `commits` commits over `commits / 2` paths (at least 64, at most 40k) and 300 people.
    #[must_use]
    pub fn sized(commits: usize) -> Self {
        Self {
            commits,
            paths: (commits / 2).clamp(64, 40_000),
            authors: 300,
            files_per_commit: 4,
            seed: 7,
        }
    }
}

const KINDS: [&str; 6] = ["feat", "fix", "docs", "refactor", "chore", "update"];

/// Builds a repository of `shape` at `dir` with `git fast-import`, one linear branch `main`.
/// Seconds even for 100k commits. Identical input gives identical oids.
pub fn build(dir: &Path, shape: Shape) {
    std::fs::create_dir_all(dir).expect("create dir");
    git(dir, &["init", "-q", "-b", "main"]);
    let mut rng = Rng::new(shape.seed);
    let mut stream = String::new();
    let start: i64 = 1_420_070_400; // 2015-01-01
    for i in 0..shape.commits {
        let who = rng.below(shape.authors as u64);
        let (name, email) = if who % 10 == 9 {
            (
                format!("dep{who}[bot]"),
                format!("dep{who}[bot]@users.noreply"),
            )
        } else {
            (format!("Person {who}"), format!("p{who}@example.org"))
        };
        let time = start + i as i64 * 3600;
        let kind = KINDS[rng.below(KINDS.len() as u64) as usize];
        let scope = rng.below(8);
        let msg = format!("{kind}(s{scope}): change {i}\n");
        write!(
            stream,
            "commit refs/heads/main\nmark :{}\nauthor {name} <{email}> {time} +0000\n\
             committer {name} <{email}> {time} +0000\ndata {}\n{msg}",
            i + 1,
            msg.len()
        )
        .expect("write string");
        if i > 0 {
            writeln!(stream, "from :{i}").expect("write string");
        }
        for _ in 0..shape.files_per_commit {
            let p = rng.below(shape.paths as u64);
            let mut body = String::new();
            for l in 0..=rng.below(12) {
                writeln!(body, "{l} {i}").expect("write string");
            }
            write!(
                stream,
                "M 100644 inline d{}/d{}/f{p}.rs\ndata {}\n{body}\n",
                p % 37,
                p % 211,
                body.len()
            )
            .expect("write string");
        }
    }
    git_stdin(dir, &["fast-import", "--quiet"], stream.as_bytes());
    git(dir, &["reset", "-q", "--hard", "main"]);
}

#[cfg(test)]
mod tests {
    use crate::synthetic::{Rng, Shape, build};

    #[test]
    fn same_seed_same_tip() {
        let a = tempfile_dir("a");
        let b = tempfile_dir("b");
        build(&a, Shape::sized(50));
        build(&b, Shape::sized(50));
        let tip = |d: &std::path::Path| crate::git(d, &["rev-parse", "HEAD"]);
        assert_eq!(tip(&a), tip(&b));
        assert_eq!(crate::git(&a, &["rev-list", "--count", "HEAD"]), "50");
        std::fs::remove_dir_all(a.parent().expect("parent")).expect("clean");
        assert_ne!(Rng::new(1).next(), Rng::new(2).next());
    }

    fn tempfile_dir(n: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("gt-synth-{}", std::process::id()))
            .join(n)
    }
}
