use std::path::{Path, PathBuf};

use crate::{git, git_at, git_stdin, git_stdin_at};

/// Author time of the commit dated far past any calendar the charts know: year ~950 million.
pub const FAR_FUTURE: i64 = 30_000_000_000_000_000;

/// Every history shape that has broken an insight once, in one repository, plus a shallow
/// clone of it. Built under `dir`:
///
/// - two roots joined by an octopus merge of three parents;
/// - an early author `jdoe <old@x.io>` that a `.mailmap` added later maps to
///   `Jane Doe <jane@x.io>`, who then commits under the new address;
/// - a commit dated [`FAR_FUTURE`] and one with author time `i64::MIN` and committer time
///   `i64::MAX`, written with `hash-object --literally` as git refuses such dates;
/// - an ordinary tip after them;
/// - `.git/info` as a symlink to a directory outside the repository (unix);
/// - `shallow`: a `--depth 3` clone whose boundary is the [`FAR_FUTURE`] commit.
pub struct Adversarial {
    pub repo: PathBuf,
    pub shallow: PathBuf,
    /// The two parentless commits, oldest first.
    pub roots: [String; 2],
    pub octopus: String,
    pub mailmap: String,
    pub far: String,
    pub extreme: String,
    pub tip: String,
}

impl Adversarial {
    #[must_use]
    pub fn build(dir: &Path) -> Self {
        let repo = dir.join("repo");
        let f = Fixture::init(&repo).at("2024-01-01T12:00:00Z");
        f.git(&["config", "user.name", "jdoe"]);
        f.git(&["config", "user.email", "old@x.io"]);
        let f = f.write("a.txt", "a\n").commit("feat: first root").branch("side");
        let f = f.write("s.txt", "s\n").commit("feat(side): side").checkout("main");
        let f = f.write("m.txt", "m\n").commit("fix: main");
        let root_a = f.git(&["rev-list", "--max-parents=0", "HEAD"]);
        let (root_b, octopus) = octopus_with_second_root(&f);
        f.git(&["reset", "-q", "--hard", &octopus]);
        f.git(&["config", "user.name", "Jane Doe"]);
        f.git(&["config", "user.email", "jane@x.io"]);
        let f = f
            .at("2024-02-01T12:00:00Z")
            .write(".mailmap", "Jane Doe <jane@x.io> <old@x.io>\n")
            .commit("docs: mailmap");
        let mailmap = f.git(&["rev-parse", "HEAD"]);
        let tree = f.git(&["rev-parse", concat!("HEAD^", "{tree}")]);
        let raw = |parent: &str, author: &str, committer: &str, msg: &str| {
            literal_commit(&f, &tree, parent, author, committer, msg)
        };
        let far =
            raw(&mailmap, &format!("{FAR_FUTURE} +0000"), &format!("{FAR_FUTURE} +0000"), "far");
        let extreme =
            raw(&far, &format!("{} -9959", i64::MIN), &format!("{} +9959", i64::MAX), "extreme");
        f.git(&["reset", "-q", "--hard", &extreme]);
        let f = f.at("2024-03-01T12:00:00Z").write("a.txt", "a\nb\n").commit("fix: tip");
        let tip = f.git(&["rev-parse", "HEAD"]);
        f.git(&["branch", "-q", "-D", "side"]);
        #[cfg(unix)]
        {
            let outside = dir.join("info-elsewhere");
            std::fs::rename(repo.join(".git/info"), &outside).expect("move info");
            std::os::unix::fs::symlink(&outside, repo.join(".git/info")).expect("link info");
        }
        let shallow = dir.join("shallow");
        let url = format!("file://{}", repo.display());
        git(dir, &["clone", "-q", "--depth", "3", &url, shallow.to_str().expect("utf8")]);
        Self { repo, shallow, roots: [root_a, root_b], octopus, mailmap, far, extreme, tip }
    }
}

/// A parentless commit holding `o.txt`, then a three-parent merge of `HEAD`, `side` and it.
/// Returns (second root, octopus).
fn octopus_with_second_root(f: &Fixture) -> (String, String) {
    let blob = git_stdin(f.path(), &["hash-object", "-w", "--stdin"], b"o\n");
    let entry = format!("100644 blob {blob}\to.txt\n");
    let lone = git_stdin(f.path(), &["mktree"], entry.as_bytes());
    let root = f.git(&["commit-tree", &lone, "-m", "feat: second root"]);
    let merged = f.git(&["merge-tree", "--write-tree", "HEAD", "side"]);
    let mut listing = f.git(&["ls-tree", &merged]);
    listing.push('\n');
    listing.push_str(&entry);
    let both = git_stdin(f.path(), &["mktree"], listing.as_bytes());
    let octopus = f.git(&[
        "commit-tree",
        &both,
        "-p",
        "HEAD",
        "-p",
        "side",
        "-p",
        &root,
        "-m",
        "merge three",
    ]);
    (root, octopus)
}

/// A commit object written as given: git refuses dates outside its range on `commit`.
fn literal_commit(
    f: &Fixture,
    tree: &str,
    parent: &str,
    author: &str,
    committer: &str,
    msg: &str,
) -> String {
    let body = format!(
        "tree {tree}\nparent {parent}\nauthor Jane Doe <jane@x.io> {author}\n\
         committer Jane Doe <jane@x.io> {committer}\n\n{msg}\n"
    );
    git_stdin(
        f.path(),
        &["hash-object", "-t", "commit", "--literally", "-w", "--stdin"],
        body.as_bytes(),
    )
}

/// A repository on disk built step by step. Every step returns `self`.
pub struct Fixture {
    dir: PathBuf,
    date: Option<String>,
}

impl Fixture {
    /// `git init -b main` in `dir`, which is created if missing.
    pub fn init(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).expect("create dir");
        let f = Self { dir: dir.to_owned(), date: None };
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

    pub fn git(&self, args: &[&str]) -> String {
        self.date.as_ref().map_or_else(|| git(&self.dir, args), |d| git_at(&self.dir, d, args))
    }

    /// Writes `body` to `path`, creating parent directories.
    #[must_use]
    pub fn write(self, path: &str, body: &str) -> Self {
        let full = self.dir.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
        std::fs::write(full, body).expect("write");
        self
    }

    /// Stages everything and commits it with `msg` as `author` (`"Name <email>"`).
    #[must_use]
    pub fn commit_by(self, author: &str, msg: &str) -> Self {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--author", author, "-m", msg]);
        self
    }

    /// Stages everything and commits it with `msg`.
    #[must_use]
    pub fn commit(self, msg: &str) -> Self {
        self.git(&["add", "-A"]);
        // The message on stdin, not `-m`: Windows caps a command line at 32 KB, which a
        // fixture with a block-sized message passed.
        git_stdin_at(
            &self.dir,
            self.date.as_deref(),
            &["commit", "-q", "--allow-empty", "-F", "-"],
            msg.as_bytes(),
        );
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

    /// The repository already at `dir`, for tests that add commits to one they built.
    pub fn open(dir: &Path) -> Self {
        Self { dir: dir.to_owned(), date: None }
    }
}
