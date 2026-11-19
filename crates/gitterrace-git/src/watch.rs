//! A cheap change detector for a repository: one `u64` that flips whenever HEAD, a ref, the index,
//! a half-done operation, the stash or a linked worktree changes.
//!
//! The hot path is `stat` only. It never opens an object, a pack or a ref file, and it holds no
//! repository handle, so it cannot walk history. Git replaces files by renaming a lock file over
//! them, which gives the new file a new inode, and it adds or removes loose refs by creating and
//! renaming inside the ref's directory, which moves that directory's mtime. So a ref directory is
//! listed once and listed again only when its stamp moves; a repository with 50k loose refs costs
//! one `stat` per directory per tick, not one per ref.

use std::collections::HashMap;
use std::fs::{self, Metadata};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::Repo;

/// A directory modified this recently may be rewritten again inside the same filesystem
/// timestamp tick, which would leave its stamp unchanged. Such a directory is hashed by the
/// stamps of its entries until it has been quiet for this long. Below roughly a coarse ext4
/// tick (4 ms) a change could hide, far above 2 s only costs extra `stat` calls.
const RACY_WINDOW: Duration = Duration::from_secs(2);

/// Files under the git directory that say where HEAD is and what operation is half done.
const HEAD_FILES: [&str; 10] = [
    "HEAD",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "REBASE_HEAD",
    "BISECT_LOG",
    "rebase-merge",
    "rebase-apply",
    "sequencer",
    "AUTO_MERGE",
];

/// Files under the common git directory that hold refs besides the `refs` tree.
const REF_FILES: [&str; 2] = ["packed-refs", "reftable/tables.list"];

/// The stash ref and its reflog, which `git stash drop` rewrites without touching the ref.
const STASH_FILES: [&str; 2] = ["refs/stash", "logs/refs/stash"];

/// What one `stat` says about a path. `None` fields mean the path is absent.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Stamp {
    present: bool,
    size: u64,
    mtime_ns: u128,
    ino: u64,
}

impl Stamp {
    fn of(meta: Option<&Metadata>) -> Self {
        let Some(m) = meta else {
            return Self {
                present: false,
                size: 0,
                mtime_ns: 0,
                ino: 0,
            };
        };
        Self {
            present: true,
            size: m.len(),
            mtime_ns: mtime_ns(m),
            ino: inode(m),
        }
    }
}

fn mtime_ns(m: &Metadata) -> u128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

#[cfg(unix)]
fn inode(m: &Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::ino(m)
}

#[cfg(not(unix))]
const fn inode(_: &Metadata) -> u64 {
    0
}

struct Listed {
    stamp: Stamp,
    children: Vec<PathBuf>,
    /// Hash of the entries' stamps, taken while the directory was racy. Kept after it goes quiet
    /// so the fingerprint does not flip when the window closes.
    entries: u64,
}

/// System calls one tick made. Tests assert on it; the watcher itself never reads it.
#[cfg(test)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cost {
    pub(crate) stats: u64,
    pub(crate) dir_listings: u64,
}

/// One hash per kind of change, so a page can react to what it shows. Equal parts mean that
/// kind did not move.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    /// HEAD itself, or an operation marker (merge, rebase, cherry-pick, revert, bisect).
    pub head: u64,
    /// Any branch, tag or remote ref, loose or packed. A commit moves this.
    pub refs: u64,
    /// The stash.
    pub stash: u64,
    /// The list of linked worktrees, or one worktree's HEAD or index.
    pub worktrees: u64,
    /// The main index: staging, and most commands that touch files.
    pub index: u64,
}

impl Parts {
    /// Names of the parts that differ from `before`, in a fixed order.
    pub fn changed_since(&self, before: &Self) -> Vec<&'static str> {
        [
            ("head", self.head != before.head),
            ("refs", self.refs != before.refs),
            ("stash", self.stash != before.stash),
            ("worktrees", self.worktrees != before.worktrees),
            ("index", self.index != before.index),
        ]
        .into_iter()
        .filter_map(|(n, moved)| moved.then_some(n))
        .collect()
    }

    /// One value for all parts: it differs whenever any part does.
    pub fn digest(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.head, self.refs, self.stash, self.worktrees, self.index).hash(&mut h);
        h.finish()
    }
}

/// Fingerprints one repository. Keep it between ticks: it remembers which ref directories exist.
pub struct Watcher {
    git_dir: PathBuf,
    common_dir: PathBuf,
    dirs: HashMap<PathBuf, Listed>,
    racy: Duration,
    #[cfg(test)]
    cost: Cost,
}

impl Repo {
    /// A change detector for this repository. Every tick of it is `stat` calls only.
    pub fn watcher(&self) -> Watcher {
        Watcher::new(
            self.git_dir().to_path_buf(),
            self.repo().common_dir().to_path_buf(),
        )
    }
}

impl Watcher {
    pub(crate) fn new(git_dir: PathBuf, common_dir: PathBuf) -> Self {
        Self {
            git_dir,
            common_dir,
            dirs: HashMap::new(),
            racy: RACY_WINDOW,
            #[cfg(test)]
            cost: Cost::default(),
        }
    }

    /// System calls made by the last [`Watcher::fingerprint`].
    #[cfg(test)]
    pub(crate) const fn last_cost(&self) -> Cost {
        self.cost
    }

    /// The current fingerprint. Equal values mean nothing a page shows has changed; a different
    /// value means something may have.
    pub fn fingerprint(&mut self) -> u64 {
        self.parts().digest()
    }

    /// The fingerprint split by kind of change. Same cost as [`Watcher::fingerprint`].
    pub fn parts(&mut self) -> Parts {
        #[cfg(test)]
        {
            self.cost = Cost::default();
        }
        let now = SystemTime::now();
        let hasher = || std::collections::hash_map::DefaultHasher::new();
        let (mut head, mut refs, mut stash, mut worktrees, mut index) =
            (hasher(), hasher(), hasher(), hasher(), hasher());

        for name in HEAD_FILES {
            self.stamp(&self.git_dir.join(name)).hash(&mut head);
        }
        self.stamp(&self.git_dir.join("index")).hash(&mut index);
        for name in REF_FILES {
            self.stamp(&self.common_dir.join(name)).hash(&mut refs);
        }
        for name in STASH_FILES {
            self.stamp(&self.common_dir.join(name)).hash(&mut stash);
        }
        let mut roots = vec![self.common_dir.join("refs")];
        if self.git_dir != self.common_dir {
            roots.push(self.git_dir.join("refs"));
        }
        for root in roots {
            self.hash_tree(&root, now, &mut refs);
        }
        self.hash_worktrees(&mut worktrees);
        Parts {
            head: head.finish(),
            refs: refs.finish(),
            stash: stash.finish(),
            worktrees: worktrees.finish(),
            index: index.finish(),
        }
    }

    // `self` carries the test-only cost counters; without them it is unused.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            clippy::needless_pass_by_ref_mut,
            reason = "the test-only cost counters need &mut self"
        )
    )]
    fn stamp(&mut self, path: &Path) -> Stamp {
        #[cfg(test)]
        {
            self.cost.stats += 1;
        }
        Stamp::of(fs::symlink_metadata(path).ok().as_ref())
    }

    /// Walks the ref directories, listing a directory only when its stamp moved or is racy.
    fn hash_tree(&mut self, root: &Path, now: SystemTime, h: &mut impl Hasher) {
        let mut seen = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let stamp = self.stamp(&dir);
            stamp.hash(h);
            if !stamp.present {
                continue;
            }
            seen.push(dir.clone());
            let racy = is_racy(stamp, now, self.racy);
            let fresh = self.dirs.get(&dir).is_some_and(|l| l.stamp == stamp) && !racy;
            if !fresh {
                let (children, entries) = self.list(&dir, racy);
                self.dirs.insert(
                    dir.clone(),
                    Listed {
                        stamp,
                        children,
                        entries,
                    },
                );
            }
            if let Some(l) = self.dirs.get(&dir) {
                l.entries.hash(h);
                stack.extend(l.children.iter().rev().cloned());
            }
        }
        self.dirs
            .retain(|p, _| !p.starts_with(root) || seen.contains(p));
    }

    /// Subdirectories of `dir`, sorted, and for a racy directory a hash of its entries' stamps.
    // `self` carries the test-only cost counters; without them it is unused.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            clippy::needless_pass_by_ref_mut,
            reason = "the test-only cost counters need &mut self"
        )
    )]
    fn list(&mut self, dir: &Path, with_entries: bool) -> (Vec<PathBuf>, u64) {
        #[cfg(test)]
        {
            self.cost.dir_listings += 1;
        }
        let mut children = Vec::new();
        let mut entries: Vec<(std::ffi::OsString, Stamp)> = Vec::new();
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.filter_map(std::result::Result::ok) {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    children.push(e.path());
                }
                if with_entries {
                    #[cfg(test)]
                    {
                        self.cost.stats += 1;
                    }
                    entries.push((e.file_name(), Stamp::of(e.metadata().ok().as_ref())));
                }
            }
        }
        children.sort();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        entries.hash(&mut h);
        (children, h.finish())
    }

    /// Each linked worktree's own HEAD and index, which sit outside the main HEAD.
    fn hash_worktrees(&mut self, h: &mut impl Hasher) {
        let base = self.common_dir.join("worktrees");
        let Some(wt) = self.dirs_listing(&base) else {
            return;
        };
        self.stamp(&base).hash(h);
        for id in wt {
            self.stamp(&base.join(&id).join("HEAD")).hash(h);
            self.stamp(&base.join(&id).join("index")).hash(h);
        }
    }

    /// Worktree ids, re-listed only when the `worktrees` directory moves.
    fn dirs_listing(&mut self, base: &Path) -> Option<Vec<std::ffi::OsString>> {
        let stamp = self.stamp(base);
        if !stamp.present {
            self.dirs.remove(base);
            return None;
        }
        let racy = is_racy(stamp, SystemTime::now(), self.racy);
        let cached = self.dirs.get(base).is_some_and(|l| l.stamp == stamp) && !racy;
        if !cached {
            let (children, entries) = self.list(base, false);
            self.dirs.insert(
                base.to_path_buf(),
                Listed {
                    stamp,
                    children,
                    entries,
                },
            );
        }
        Some(
            self.dirs
                .get(base)?
                .children
                .iter()
                .filter_map(|c| c.file_name().map(std::ffi::OsStr::to_os_string))
                .collect(),
        )
    }
}

fn is_racy(stamp: Stamp, now: SystemTime, window: Duration) -> bool {
    let modified =
        SystemTime::UNIX_EPOCH + Duration::from_nanos(u64::try_from(stamp.mtime_ns).unwrap_or(0));
    now.duration_since(modified).is_ok_and(|age| age < window) || modified > now
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;
    use std::fs;
    use std::path::Path;

    use super::Watcher;
    use crate::Repo;

    fn commit(dir: &Path, file: &str, body: &str) {
        fs::write(dir.join(file), body).expect("write");
        git(dir, &["add", "."]);
        git(dir, &["commit", "-qm", body]);
    }

    fn quiet(repo: &Repo) -> Watcher {
        let mut w = repo.watcher();
        w.racy = std::time::Duration::ZERO;
        w
    }

    fn base() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().expect("tmp");
        let dir = tmp.path().join("r");
        fs::create_dir(&dir).expect("mkdir");
        git(&dir, &["init", "-q", "-b", "main"]);
        commit(&dir, "a", "one");
        (tmp, dir)
    }

    #[test]
    fn every_change_kind_flips_the_fingerprint() {
        let (_tmp, dir) = base();
        let repo = Repo::discover(&dir).expect("open");
        let mut w = quiet(&repo);
        let mut last = w.fingerprint();
        let step = |what: &str, w: &mut Watcher, last: &mut u64| {
            let now = w.fingerprint();
            assert_ne!(now, *last, "{what} must flip the fingerprint");
            *last = now;
        };

        commit(&dir, "a", "two");
        step("commit", &mut w, &mut last);
        git(&dir, &["branch", "side"]);
        step("branch", &mut w, &mut last);
        git(&dir, &["branch", "feature/deep/x"]);
        step("branch in a new subdirectory", &mut w, &mut last);
        git(&dir, &["branch", "feature/deep/y"]);
        step("branch in an existing subdirectory", &mut w, &mut last);
        git(&dir, &["tag", "v1"]);
        step("tag", &mut w, &mut last);
        git(&dir, &["pack-refs", "--all", "--prune"]);
        step("packing refs", &mut w, &mut last);
        git(&dir, &["branch", "-D", "side"]);
        step("deleting a packed branch", &mut w, &mut last);
        git(&dir, &["checkout", "-q", "feature/deep/x"]);
        step("checkout", &mut w, &mut last);
        fs::write(dir.join("a"), "dirty").expect("write");
        git(&dir, &["stash", "-q"]);
        step("stash", &mut w, &mut last);
        fs::write(dir.join("b"), "new").expect("write");
        git(&dir, &["add", "b"]);
        step("staging", &mut w, &mut last);
        fs::write(dir.join(".git/MERGE_HEAD"), "x\n").expect("write");
        step("operation marker", &mut w, &mut last);
        fs::remove_file(dir.join(".git/MERGE_HEAD")).expect("rm");
        step("operation marker removed", &mut w, &mut last);
        let wt = dir.parent().expect("parent").join("wt");
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                wt.to_str().expect("utf8"),
                "-b",
                "wtb",
            ],
        );
        step("worktree added", &mut w, &mut last);
        commit(&wt, "w", "in worktree");
        step("commit in a linked worktree", &mut w, &mut last);
        git(
            &dir,
            &["worktree", "remove", "--force", wt.to_str().expect("utf8")],
        );
        step("worktree removed", &mut w, &mut last);
    }

    #[test]
    fn parts_name_what_moved() {
        let (_tmp, dir) = base();
        let repo = Repo::discover(&dir).expect("open");
        let mut w = quiet(&repo);
        let mut last = w.parts();
        let mut moved = |w: &mut Watcher| {
            let now = w.parts();
            let names = now.changed_since(&last);
            last = now;
            names
        };
        fs::write(dir.join("b"), "x").expect("write");
        git(&dir, &["add", "b"]);
        assert_eq!(moved(&mut w), ["index"], "staging moves the index only");
        git(&dir, &["commit", "-qm", "two"]);
        let names = moved(&mut w);
        assert!(
            names.contains(&"refs") && !names.contains(&"head"),
            "{names:?}"
        );
        git(&dir, &["checkout", "-q", "--detach"]);
        assert!(moved(&mut w).contains(&"head"), "detaching moves HEAD");
        fs::write(dir.join("a"), "dirty").expect("write");
        git(&dir, &["stash", "-q"]);
        assert!(moved(&mut w).contains(&"stash"));
        let wt = dir.parent().expect("parent").join("wt2");
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-q",
                wt.to_str().expect("utf8"),
                "-b",
                "w2",
            ],
        );
        assert!(moved(&mut w).contains(&"worktrees"));
        assert!(moved(&mut w).is_empty(), "a quiet tick names nothing");
    }

    #[test]
    fn unchanged_repository_is_stable_and_stat_only() {
        let (_tmp, dir) = base();
        git(&dir, &["branch", "feature/x"]);
        let repo = Repo::discover(&dir).expect("open");
        let mut w = quiet(&repo);
        let first = w.fingerprint();
        assert!(
            w.last_cost().dir_listings > 0,
            "the first tick lists the ref tree"
        );
        for _ in 0..5 {
            assert_eq!(w.fingerprint(), first);
            assert_eq!(w.last_cost().dir_listings, 0, "a quiet tick lists nothing");
        }
    }

    #[test]
    fn recent_changes_are_not_hidden_by_a_coarse_clock() {
        let (_tmp, dir) = base();
        let repo = Repo::discover(&dir).expect("open");
        let mut w = repo.watcher();
        let a = w.fingerprint();
        git(&dir, &["branch", "one"]);
        let b = w.fingerprint();
        git(&dir, &["branch", "two"]);
        let c = w.fingerprint();
        assert!(a != b && b != c, "back to back ref changes are all seen");
    }

    #[test]
    fn a_vanished_ref_directory_is_forgotten() {
        let (_tmp, dir) = base();
        git(&dir, &["branch", "gone/x"]);
        let repo = Repo::discover(&dir).expect("open");
        let mut w = quiet(&repo);
        let with = w.fingerprint();
        git(&dir, &["branch", "-D", "gone/x"]);
        fs::remove_dir_all(dir.join(".git/refs/heads/gone")).ok();
        assert_ne!(w.fingerprint(), with);
        assert!(!w.dirs.keys().any(|p| p.ends_with("gone")));
    }
}
