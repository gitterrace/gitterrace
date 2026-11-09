//! Related repositories: the other served repositories that share a root commit with this one,
//! forks and clones of one project served side by side. The log's toolbar links them.
//!
//! Only what the table serves takes part: a peer is added when a repository joins and goes
//! when it leaves, so a name shown here is one the repository index lists too, and no path
//! ever reaches a page.
//!
//! Each peer keeps its [`Roots`], warmed off the request path: once when it joins the table,
//! and again from the index refresher when refs move. A page only reads what is cached, so a
//! cold peer shares nothing and the line shows once the warm-up ends. No lock is held across
//! git work: a warm-up copies the set out, updates it, then stores it back.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use gitterrace_git::{Repo, Roots};

/// How often the refresher of one repository lists the refs of the other peers again. Listing
/// refs is cheap, and a walk runs only when they moved.
const REWARM: Duration = Duration::from_secs(30);

/// One served repository as the related links see it: its URL name and where it lives.
pub(crate) struct Peer {
    pub(crate) name: String,
    path: PathBuf,
    /// The cached set and when it was last warmed. Locked only to copy in or out.
    roots: Mutex<(Option<Instant>, Roots)>,
    /// A warm-up is running: a second one skips instead of walking twice.
    busy: AtomicBool,
    /// Warm-ups run so far, for the tests that prove a page never runs one.
    #[cfg(test)]
    walks: std::sync::atomic::AtomicUsize,
}

/// Every served repository, in the table's order. Shared by every app of one server.
#[derive(Clone, Default)]
pub(crate) struct Peers(Arc<RwLock<Vec<Arc<Peer>>>>);

impl Peers {
    /// Adds the repository the table now serves as `name`. Nothing is walked here: a
    /// refresher warms it through [`Peers::warm`] once its own repository id is known, so the
    /// root walks never hold up the id that permalinks carry.
    pub(crate) fn add(&self, name: &str, path: PathBuf) {
        let peer = Arc::new(Peer {
            name: name.to_owned(),
            path,
            roots: Mutex::new((None, Roots::default())),
            busy: AtomicBool::new(false),
            #[cfg(test)]
            walks: std::sync::atomic::AtomicUsize::new(0),
        });
        if let Ok(mut g) = self.0.write() {
            g.retain(|p| p.name != name);
            g.push(peer);
        }
    }

    /// Drops `name`, taken off the table.
    pub(crate) fn remove(&self, name: &str) {
        if let Ok(mut g) = self.0.write() {
            g.retain(|p| p.name != name);
        }
    }

    fn snapshot(&self) -> Vec<Arc<Peer>> {
        self.0.read().map(|g| g.clone()).unwrap_or_default()
    }

    /// Warms `own` from its open `repo`, and every other peer not warmed within [`REWARM`].
    /// Blocking git work: the index refresher calls it off the request path.
    /// Every peer walks through its own handle opened from its path, never the page's `repo`:
    /// a walk then shares no handle, cache or lock with a page being built.
    pub(crate) fn warm(&self, own: &str) {
        let now = Instant::now();
        for p in self.snapshot() {
            if p.name == own || p.warmed().is_none_or(|at| now.duration_since(at) >= REWARM) {
                p.warm();
            }
        }
    }

    /// The names of the served repositories other than `own` that share a root commit with
    /// it, in table order, from the cached sets only: no git work, no waiting on a warm-up.
    /// Empty when `own` is not in the table (a lone repository) or not warm yet.
    pub(crate) fn related(&self, own: &str) -> Vec<String> {
        let peers = self.snapshot();
        let Some(me) = peers.iter().find(|p| p.name == own) else {
            return Vec::new();
        };
        let mine = me.cached();
        if mine.ids().is_empty() {
            return Vec::new();
        }
        peers
            .iter()
            .filter(|p| p.name != own && p.cached().shares(&mine))
            .map(|p| p.name.clone())
            .collect()
    }

    /// Warm-ups run so far on the peers in the table.
    #[cfg(test)]
    fn walks(&self) -> usize {
        self.snapshot()
            .iter()
            .map(|p| p.walks.load(Ordering::Relaxed))
            .sum()
    }
}

impl Peer {
    /// The cached set, or none while a warm-up holds the lock or it is cold.
    fn cached(&self) -> Roots {
        self.roots
            .try_lock()
            .map(|g| g.1.clone())
            .unwrap_or_default()
    }

    /// When the set was last stored. Never waits either: a set being stored counts as fresh.
    fn warmed(&self) -> Option<Instant> {
        self.roots
            .try_lock()
            .map_or_else(|_| Some(Instant::now()), |g| g.0)
    }

    /// Updates the set from a handle opened from its path for this walk alone. A repository
    /// that will not open shares nothing.
    fn warm(&self) {
        if self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut roots = self.roots.lock().map(|g| g.1.clone()).unwrap_or_default();
        let repo = Repo::discover(&self.path).ok();
        #[cfg(test)]
        self.walks.fetch_add(1, Ordering::Relaxed);
        #[cfg(test)]
        hold::wait(&self.path);
        match repo.as_ref().map(|r| roots.update(r)) {
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                tracing::debug!("roots of {}: {e}", self.name);
                roots = Roots::default();
            }
            None => roots = Roots::default(),
        }
        if let Ok(mut g) = self.roots.lock() {
            *g = (Some(Instant::now()), roots);
        }
        self.busy.store(false, Ordering::Release);
    }
}

/// A test hold on root walks: a walk of a held path waits before reading git, so a test can
/// prove a page never waits for one. Keyed by path, so parallel tests are not held.
#[cfg(test)]
pub(crate) mod hold {
    use std::path::{Path, PathBuf};
    use std::sync::{Condvar, Mutex};

    static HELD: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    static FREED: Condvar = Condvar::new();

    pub(crate) fn set(path: &Path, on: bool) {
        if let Ok(mut g) = HELD.lock() {
            g.retain(|p| p != path);
            if on {
                g.push(path.to_owned());
            }
        }
        FREED.notify_all();
    }

    pub(crate) fn wait(path: &Path) {
        let Ok(mut g) = HELD.lock() else { return };
        while g.iter().any(|p| p == path) {
            let Ok(next) = FREED.wait(g) else { return };
            g = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use gitterrace_testkit::git;
    use std::path::Path;

    use super::Peers;

    fn repo(d: &Path, msg: &str) {
        std::fs::create_dir_all(d).expect("dir");
        git(d, &["init", "-q", "-b", "main"]);
        git(d, &["commit", "-q", "--allow-empty", "-m", msg]);
    }

    /// A clone is related, a stranger is not, the repository itself is never listed, and a
    /// peer taken off the table is gone at once. Cold peers relate to nothing, and reading
    /// the related names never walks: only a warm-up does.
    #[test]
    fn only_warm_served_peers_that_share_a_root_are_related() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (a, b, c) = (
            tmp.path().join("a"),
            tmp.path().join("b"),
            tmp.path().join("c"),
        );
        repo(&a, "root");
        git(
            tmp.path(),
            &[
                "clone",
                "-q",
                a.to_str().expect("utf8"),
                b.to_str().expect("utf8"),
            ],
        );
        repo(&c, "other");
        let peers = Peers::default();
        for (n, p) in [("a", &a), ("b", &b), ("c", &c)] {
            peers.add(n, p.clone());
        }
        let before = peers.walks();
        assert!(peers.related("a").is_empty(), "cold: no line yet");
        assert_eq!(peers.walks(), before, "a read never walks");
        peers.warm("a");
        assert!(peers.walks() >= before + 3, "the warm-up walks");
        let walked = peers.walks();
        assert_eq!(peers.related("a"), ["b"]);
        assert!(peers.related("c").is_empty());
        assert!(peers.related("lone").is_empty(), "not in the table");
        assert_eq!(peers.walks(), walked, "reads never walk");
        peers.remove("b");
        assert!(peers.related("a").is_empty());
    }
}
