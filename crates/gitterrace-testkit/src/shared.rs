//! Repositories built once per test process and shared read-only, so a test that only reads
//! pages does not pay for its own `git init`. A test that writes to its repository builds its
//! own with [`crate::Fixture`] instead.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::Fixture;

/// The shapes tests share.
///
/// Each is built on first use and lives until the process exits. A
/// router writes its cache under the repository's `.git`, so a test binary opens one router per
/// shared repository.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shared {
    /// [`Fixture::standard`]: README, `src/a.rs`, `feat(core): first` and `fix: second`.
    Standard,
    /// Documents in every place a card shows one.
    ///
    /// A README with headings at the root and in
    /// `docs/`, a licence, a contributing guide, a 250-line text file and `src/a.rs`, tagged
    /// `v1` on the first of two commits.
    Docs,
    /// Two authors over three dated commits.
    ///
    /// Bob's `feat: a` on 2024-01-10 and `fix: a` on
    /// 2024-03-10 in `src/a.rs`, then Ann's `docs: guide` on 2024-03-20 in `docs/guide.md`.
    Dated,
}

impl Shared {
    /// The repository's worktree, built on the first call.
    #[must_use]
    pub fn path(self) -> &'static Path {
        static DIRS: [OnceLock<PathBuf>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
        DIRS[self as usize].get_or_init(|| {
            // Kept for the life of the process: the shared repository outlives every test.
            let dir = tempfile::tempdir().expect("tmp").keep().join("repo");
            self.build(&dir);
            dir
        })
    }

    fn build(self, dir: &Path) {
        match self {
            Self::Standard => {
                Fixture::standard(dir);
            }
            Self::Docs => docs(dir),
            Self::Dated => dated(dir),
        }
    }
}

fn docs(dir: &Path) {
    let doc = "# T\n\n## A\n\n## B\n\n## C\n";
    let f = Fixture::init(dir)
        .write("README.md", doc)
        .write("LICENSE", "MIT\n")
        .write("CONTRIBUTING.md", doc)
        .write("docs/README.md", doc)
        .write("long.txt", &"line\n".repeat(250))
        .write("src/a.rs", "fn main() {}\n")
        .commit("feat(core): first")
        .tag("v1", "first release");
    let _second = f
        .write("src/a.rs", "fn main() {\n}\n")
        .commit("fix: second");
}

fn dated(dir: &Path) {
    let mut f = Fixture::init(dir);
    for (who, date, path, msg) in [
        (
            "Bob <bob@y.io>",
            "2024-01-10T12:00:00Z",
            "src/a.rs",
            "feat: a",
        ),
        (
            "Bob <bob@y.io>",
            "2024-03-10T12:00:00Z",
            "src/a.rs",
            "fix: a",
        ),
        (
            "Ann <ann@x.io>",
            "2024-03-20T12:00:00Z",
            "docs/guide.md",
            "docs: guide",
        ),
    ] {
        f = f.at(date).write(path, &format!("{msg}\nmore\n"));
        f.git(&["add", "-A"]);
        f.git(&["commit", "-q", "--author", who, "-m", msg]);
    }
}

/// A table of inputs and what each must give, checked row by row.
///
/// Every failing row is reported, not only the first:
///
/// ```
/// use gitterrace_testkit::cases;
/// cases!(|s: &str| s.len(), ["" => 0, "ab" => 2]);
/// ```
#[macro_export]
macro_rules! cases {
    ($f:expr, [$($input:expr => $want:expr),+ $(,)?]) => {{
        let f = $f;
        let mut fails: ::std::vec::Vec<::std::string::String> = ::std::vec::Vec::new();
        $(
            let got = f($input);
            if got != $want {
                fails.push(format!(
                    "{} => {:?}, want {:?}",
                    stringify!($input),
                    got,
                    $want
                ));
            }
        )+
        assert!(fails.is_empty(), "{}", fails.join("\n"));
    }};
}

#[cfg(test)]
mod tests {
    use super::Shared;

    #[test]
    fn a_shared_repository_is_built_once() {
        let a = Shared::Dated.path();
        assert_eq!(a, Shared::Dated.path());
        let log = crate::git(a, &["log", "--format=%an %s"]);
        assert_eq!(log, "Ann docs: guide\nBob fix: a\nBob feat: a");
        assert_ne!(a, Shared::Docs.path());
    }

    #[test]
    #[should_panic(expected = "\"ab\" => 2, want 3")]
    fn a_wrong_case_names_its_input() {
        cases!(|s: &str| s.len(), ["" => 0, "ab" => 3]);
    }
}
