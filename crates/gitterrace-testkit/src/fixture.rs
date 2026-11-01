use std::path::{Path, PathBuf};

use crate::{git, git_at};

/// A repository on disk built step by step. Every step returns `self`.
pub struct Fixture {
    dir: PathBuf,
    date: Option<String>,
}

impl Fixture {
    /// `git init -b main` in `dir`, which is created if missing.
    pub fn init(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).expect("create dir");
        let f = Self {
            dir: dir.to_owned(),
            date: None,
        };
        f.git(&["init", "-q", "-b", "main"]);
        f
    }

    /// Pins author and committer date for every later git call.
    #[must_use]
    pub fn at(mut self, date: &str) -> Self {
        self.date = Some(date.to_owned());
        self
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Runs git in the repository and returns trimmed stdout.
    pub fn git(&self, args: &[&str]) -> String {
        match &self.date {
            Some(d) => git_at(&self.dir, d, args),
            None => git(&self.dir, args),
        }
    }

    /// Writes `body` to `path`, creating parent directories.
    #[must_use]
    pub fn write(self, path: &str, body: &str) -> Self {
        let full = self.dir.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(full, body).expect("write");
        self
    }

    /// Stages everything and commits it with `msg`.
    #[must_use]
    pub fn commit(self, msg: &str) -> Self {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
        self
    }

    /// Creates `name` at HEAD and checks it out.
    #[must_use]
    pub fn branch(self, name: &str) -> Self {
        self.git(&["checkout", "-q", "-b", name]);
        self
    }

    #[must_use]
    pub fn checkout(self, rev: &str) -> Self {
        self.git(&["checkout", "-q", rev]);
        self
    }

    /// An annotated tag at HEAD.
    #[must_use]
    pub fn tag(self, name: &str, message: &str) -> Self {
        self.git(&["tag", "-a", name, "-m", message]);
        self
    }

    /// Detaches HEAD at its current commit.
    #[must_use]
    pub fn detach(self) -> Self {
        self.git(&["checkout", "-q", "--detach"]);
        self
    }

    /// Stashes the working tree under `message`.
    #[must_use]
    pub fn stash(self, message: &str) -> Self {
        self.git(&["stash", "push", "-q", "-m", message]);
        self
    }

    /// Adds a linked worktree at `at` on existing branch `branch`.
    #[must_use]
    pub fn worktree(self, at: &Path, branch: &str) -> Self {
        self.git(&["worktree", "add", "-q", at.to_str().expect("utf8"), branch]);
        self
    }

    /// The common shape: README, `src/a.rs`, then `feat(core): first` and `fix: second`.
    pub fn standard(dir: &Path) -> Self {
        Self::init(dir)
            .write("README.md", "# Hi\n\n[a](src/a.rs) ![i](img.png)\n")
            .write("src/a.rs", "fn main() {}\n")
            .commit("feat(core): first")
            .write("src/a.rs", "fn main() {\n}\n")
            .commit("fix: second")
    }
}
