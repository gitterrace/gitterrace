//! Incremental insights cache in `.git/gitterrace/`: the segment set, appended per update and
//! merged in the background. Statistics read it through [`feed::InsightsSource`] on demand,
//! by [`Insights`].

mod cache;
mod conventional;
pub mod feed;
mod insights;
pub mod seg;
mod source;
mod store;

#[cfg(any(test, feature = "oracle"))]
use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use gitterrace_git::Repo;

/// Re-exported so the cards crate shares the one date rules without a new dependency.
pub use gitterrace_git::{civil_from_days, days_from_civil};
pub use insights::*;
pub use source::CommitSource;
pub use store::FORMAT_VERSION;

/// Every file the cache keeps in `.git/gitterrace/`, with what it holds. The architecture page is
/// rendered from this, and a test fails when the folder holds a file not listed here.
pub const CACHE_FILES: [(&str, &str); 4] = [
    (
        store::VERSION_FILE,
        "The cache format. A mismatch deletes the cache and rebuilds it. The format is never migrated.",
    ),
    (
        store::LOCK_FILE,
        "Empty. Held locked by whichever process is extending the segment set, so two processes on one repository take turns instead of building on one manifest.",
    ),
    (
        TIP_FILE,
        "The commit id the segment files were last built for. Opening insights for that commit skips walking its history to check the files hold exactly it.",
    ),
    (
        insights::CACHE_FILE,
        "Line counts per blob id for the language chart. A blob never changes, so the counts never go stale.",
    ),
];

/// See [`CACHE_FILES`].
const TIP_FILE: &str = "tip";

/// Commits per progress step, and per call for a source that builds in batches. A batching
/// source pays a fixed cost per worker each batch (the diff attribute setup unpacks the HEAD
/// tree), so this is large enough to amortize it. The git source streams instead: its workers
/// set up once and never wait on a batch, so this only paces its progress reports.
const BATCH: usize = 2048;

/// Per-commit facts the stats pages are built from.
#[derive(Debug, Clone, Default)]
pub struct CommitRecord {
    pub id: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub offset: i32,
    pub merge: bool,
    /// Parent ids, first parent first. Lets history be followed along the first-parent chain.
    pub parents: Vec<String>,
    /// Conventional-commit type and scope, when the subject follows it.
    pub kind: Option<String>,
    pub scope: Option<String>,
    pub files: Vec<FileStat>,
}

/// One file touched by a commit, diffed against the first parent with rename detection.
#[derive(Debug, Clone, Default)]
pub struct FileStat {
    pub path: std::sync::Arc<str>,
    pub additions: u32,
    pub deletions: u32,
    /// Line count of the file after the commit (0 when deleted or binary).
    pub lines: u32,
    /// Lines in change blocks that both removed and added: the rework part of the change.
    pub rewritten: u32,
    /// Counts with whitespace-only and blank-line changes ignored. `None` when equal to the
    /// raw counts, which is nearly always.
    pub code: Option<CodeStat>,
    /// Previous path when the file was renamed in this commit.
    pub old: Option<std::sync::Arc<str>>,
    pub binary: bool,
    /// The commit deleted the file: its life ends here.
    pub deleted: bool,
}

/// Line counts that ignore whitespace-only differences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CodeStat {
    pub additions: u32,
    pub deletions: u32,
    pub rewritten: u32,
}

impl FileStat {
    /// Counts without whitespace noise, or the raw counts when nothing differs.
    pub fn code_stat(&self) -> CodeStat {
        self.code.unwrap_or(CodeStat {
            additions: self.additions,
            deletions: self.deletions,
            rewritten: self.rewritten,
        })
    }
}

/// Progress of the background build, read by the web layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

/// The source pages read when the segment set holds the shown rev.
pub type DiskSet = feed::SegmentSource<feed::DiskSource>;

/// The cache for one repository.
pub struct Index {
    dir: PathBuf,
    /// Commits a view outside the set reads from git, see [`Index::with_fresh_cap`].
    fresh_cap: usize,
    /// Bytes of segment blocks a source keeps, see [`Index::set_block_cache`].
    block_cache: std::sync::atomic::AtomicUsize,
    /// The format of a cache written by a newer build. Such a cache is never read or written.
    newer: Option<String>,
}

impl Index {
    /// Opens or creates the cache under `repo.git_dir()/gitterrace`.
    pub fn open(repo: &Repo) -> io::Result<Self> {
        Self::open_dir(repo.git_dir().join("gitterrace"))
    }

    /// Opens or creates the cache in `dir`. A format version mismatch discards old records.
    pub fn open_dir(dir: PathBuf) -> io::Result<Self> {
        let newer = store::ensure_dir(&dir)?;
        Ok(Self {
            dir,
            fresh_cap: feed::FRESH_CAP,
            block_cache: std::sync::atomic::AtomicUsize::new(feed::block_cache_default()),
            newer,
        })
    }

    /// Sets how many commits a view outside the indexed set reads from git before it asks for a
    /// reindex (`limits.unindexed_commits`; default [`feed::FRESH_CAP`]).
    #[must_use]
    pub const fn with_fresh_cap(mut self, cap: usize) -> Self {
        self.fresh_cap = cap;
        self
    }

    /// Bytes of segment blocks each source opened from now on keeps between reads: the block
    /// share of `resources.cache_mb`, set again on a reload.
    pub fn set_block_cache(&self, bytes: usize) {
        self.block_cache
            .store(bytes, std::sync::atomic::Ordering::Relaxed);
    }

    /// The cap set by [`Index::with_fresh_cap`].
    pub const fn fresh_cap(&self) -> usize {
        self.fresh_cap
    }

    /// The cache format found on disk when a newer build wrote it. Then [`Index::update`] and
    /// [`Index::pending`] fail with an explanation and leave the files alone.
    pub fn newer_format(&self) -> Option<&str> {
        self.newer.as_deref()
    }

    fn writable(&self) -> io::Result<()> {
        store::check_writable(&self.dir)
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Changes whenever the segment set does (a build, an append, a merge, a wipe and
    /// rebuild), so a cache of anything read from the set can key on it.
    pub fn generation(&self) -> u64 {
        seg::manifest::Manifest::load(&self.dir)
            .ok()
            .flatten()
            .map_or(0, |m| m.fingerprint.rotate_left(17) ^ m.generation)
    }

    /// Indexes commits reachable from `rev` that the segment set lacks: one more segment when
    /// every commit the set holds is still reachable, else a fresh build streamed from the
    /// source. A deepened or grafted clone wipes the set first (see `check_shallow`), so a
    /// boundary commit is diffed again with its parents. Waits for any other process doing the
    /// same; a commit the source cannot read is skipped and stays pending.
    pub fn update(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        progress: &dyn Fn(Progress),
    ) -> io::Result<()> {
        self.writable()?;
        let tip = source.resolve(rev);
        let order = source.rev_list(rev)?;
        let shallow = source.shallow_ids();
        // A deepened or grafted clone changes topo ranks: same rule as an unreadable cache.
        store::check_shallow(&self.dir, &shallow)?;
        // Everything below reads the set another writer may be extending: lock first.
        let _lock = store::WriteLock::take(&self.dir)?;
        // Gone before the set can change, written back once it holds exactly `tip`: a crash in
        // between leaves no marker, so the next open checks the set the slow way.
        cache::remove(&self.dir, TIP_FILE)?;
        let seg = |e: seg::SegError| io::Error::other(e.to_string());
        let previous = seg::manifest::Manifest::load(&self.dir).map_err(seg)?;
        let held = self.held()?;
        let todo = todo(&order, &held);
        if todo.is_empty() {
            if held.len() == order.len()
                && let Some(t) = tip
            {
                cache::write_atomic(&self.dir, TIP_FILE, t.as_bytes())?;
            }
            return Ok(());
        }
        let reach: HashSet<seg::Oid> = order.iter().filter_map(|id| store::oid_of(id)).collect();
        // Incremental when the set is still part of this history: the new commits become one
        // more segment. Otherwise (first build, rewritten history) everything is read again.
        let extend = previous
            .as_ref()
            .filter(|_| !held.is_empty() && held.iter().all(|o| reach.contains(o)));
        let ids: Vec<String> = if extend.is_some() {
            todo.into_iter().cloned().collect()
        } else {
            order.clone()
        };
        let total = ids.len();
        progress(Progress { done: 0, total });
        let mut read = 0_usize;
        if let Some(prev) = extend {
            // Only the new commits are held, never the history.
            let mut fresh = Vec::with_capacity(ids.len());
            source.stream_records(
                &ids,
                BATCH,
                &mut |r| {
                    read += 1;
                    fresh.push(r);
                    Ok(())
                },
                &mut |done| progress(Progress { done, total }),
            )?;
            if !fresh.is_empty() {
                let after = store::append_segment(&self.dir, prev.clone(), &shallow, &fresh)
                    .map_err(seg)?;
                let segments = after
                    .live
                    .iter()
                    .filter(|f| f.kind == seg::format::Kind::Segment as u8)
                    .count();
                if segments > store::MAX_SEGMENTS {
                    self.merge_in_background(&order, &shallow);
                }
            }
        } else {
            // Each record folds into the segment as it passes: the build never holds history.
            let mut build =
                store::SegmentBuild::new(&self.dir, previous.as_ref(), &shallow).map_err(seg)?;
            source.stream_records(
                &ids,
                BATCH,
                &mut |r| {
                    read += 1;
                    build.push(&r).map_err(seg)
                },
                &mut |done| progress(Progress { done, total }),
            )?;
            build.finish(previous, Vec::new()).map_err(seg)?;
        }
        // A skipped commit is not in the set, so it stays pending and the next update appends
        // it; until then HEAD's pages read the reach view, and the tip marker is not written.
        if read < total {
            skipped(total - read, &self.dir);
        } else if let Some(t) = tip {
            cache::write_atomic(&self.dir, TIP_FILE, t.as_bytes())?;
        }
        Ok(())
    }

    /// The commits the segment set holds, empty without a set.
    fn held(&self) -> io::Result<HashSet<seg::Oid>> {
        let seg = |e: seg::SegError| io::Error::other(e.to_string());
        let Some(store) = seg::read::Store::open(&self.dir).map_err(seg)? else {
            return Ok(HashSet::new());
        };
        Ok(store::load_order(&store)
            .map_err(seg)?
            .into_iter()
            .collect())
    }

    /// Folds the segments into one once there are more than 8, off the caller's thread, so a
    /// request never waits on it. Records come from the set itself, raw as stored. The merge
    /// takes the writer lock itself.
    #[expect(
        clippy::print_stderr,
        reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
    )]
    fn merge_in_background(&self, order: &[String], shallow: &[String]) {
        let dir = self.dir.clone();
        let (order, shallow) = (order.to_vec(), shallow.to_vec());
        let run = move || -> io::Result<bool> {
            let seg = |e: seg::SegError| io::Error::other(e.to_string());
            let Some(store) = seg::read::Store::open(&dir).map_err(seg)? else {
                return Ok(false);
            };
            let rows = feed::DiskSource::open(store, &mut |p| Ok(p.to_vec())).map_err(seg)?;
            store::merge_if_needed(&dir, &order, &shallow, Vec::new(), &mut |id| {
                let oid = store::oid_of(id)
                    .ok_or_else(|| io::Error::other(format!("merge: {id} not an oid")))?;
                rows.record_by_oid(&oid)
                    .map_err(seg)?
                    .ok_or_else(|| io::Error::other(format!("merge: {id} not stored")))
            })
        };
        let spawned = std::thread::Builder::new()
            .name("gitterrace-merge".into())
            .spawn(move || {
                if let Err(e) = run() {
                    eprintln!("gitterrace: segment merge failed: {e}");
                }
            });
        if let Err(e) = spawned {
            eprintln!("gitterrace: segment merge not started: {e}");
        }
    }

    /// Commits reachable from `rev` that [`Index::update`] would read: those the segment set
    /// lacks, or all of them once the clone's shallow boundary moved. Fails on a newer build's
    /// cache.
    pub fn pending(&self, source: &dyn CommitSource, rev: &str) -> io::Result<usize> {
        self.writable()?;
        let order = source.rev_list(rev)?;
        if store::shallow_stale(&self.dir, &source.shallow_ids()) {
            return Ok(order.len());
        }
        Ok(todo(&order, &self.held()?).len())
    }

    /// What pages read for `rev`: the segment files alone (aggregates, postings, edges from
    /// [`feed::SegmentSource`], commits, rows and strings from [`feed::DiskSource`]), so no
    /// history is held. The set must be exactly `rev`'s canonical order (the manifest holds
    /// one order, the last updated rev). `None` when it is not, or when there is no set yet:
    /// the caller then tries [`Index::rev_source`].
    pub fn disk_source(
        &self,
        source: &dyn CommitSource,
        rev: &str,
    ) -> io::Result<Option<feed::Checked<DiskSet>>> {
        // A store error opening the set is an unreadable cache, which callers rebuild.
        let seg = feed::SourceError::io;
        let open = || seg::read::Store::open(&self.dir).map_err(seg);
        let Some(store) = open()? else {
            return Ok(None);
        };
        // The set was built for this exact commit: no history walk needed to prove it holds it.
        let built = cache::read(&self.dir, TIP_FILE)
            .ok()
            .flatten()
            .is_some_and(|t| t.trim_ascii() == rev.as_bytes() && store::oid_of(rev).is_some());
        let order = if built {
            Vec::new()
        } else {
            source.rev_list(rev)?
        };
        let m = &store.manifest;
        // An unmerged set's order file is old order then new commits, not canonical, so the
        // fingerprint only matches after a merge: compare the sets.
        let same = built
            || m.commits as usize == order.len()
                && store::load_order(&store).is_ok_and(|held| {
                    let held: HashSet<seg::Oid> = held.into_iter().collect();
                    order
                        .iter()
                        .all(|id| store::oid_of(id).is_some_and(|o| held.contains(&o)))
                });
        if !same {
            return Ok(None);
        }
        let disk = feed::DiskSource::open(store, &mut |pairs| source.map_identities(rev, pairs))
            .map_err(seg)?
            .with_cache_bytes(self.block_cache.load(std::sync::atomic::Ordering::Relaxed));
        let Some(tables) = open()? else {
            return Ok(None);
        };
        Ok(Some(feed::Checked::new(std::sync::Arc::new(
            feed::SegmentSource::new(disk, tables.segments),
        ))))
    }

    /// Deletes the whole cache (an unreadable one is never repaired in place), so the next update
    /// rebuilds it. A newer build's cache is never touched.
    pub fn rebuild(&self) -> io::Result<()> {
        self.writable()?;
        let _lock = store::WriteLock::take(&self.dir)?;
        store::wipe(&self.dir)
    }

    /// The view of a commit the set does not hold exactly: the set's commits it reaches plus
    /// up to [`Index::fresh_cap`] it lacks, read from git. `tip` is `rev` resolved to a commit
    /// id. Past the cap the answer is [`feed::RevView::NeedsReindex`]: building the set for
    /// this commit would move it off HEAD, and holding its records would break the memory
    /// bound. `None` when there is no set.
    pub fn rev_source(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        tip: &str,
    ) -> io::Result<Option<feed::RevView<DiskSet>>> {
        // A store error opening the set is an unreadable cache, which callers rebuild.
        let seg = feed::SourceError::io;
        let open = || seg::read::Store::open(&self.dir).map_err(seg);
        let (Some(rows), Some(tables)) = (open()?, open()?) else {
            return Ok(None);
        };
        let disk = feed::DiskSource::open(rows, &mut |pairs| source.map_identities(rev, pairs))
            .map_err(seg)?
            .with_cache_bytes(self.block_cache.load(std::sync::atomic::Ordering::Relaxed));
        let set = feed::SegmentSource::new(disk, tables.segments);
        feed::rev_view(set, source, rev, tip, self.fresh_cap).map(Some)
    }

    /// Every commit reachable from `rev` as a record, read back from the segment set, oldest
    /// first, with the identities of `rev`'s `.mailmap` and times pinned as pages see them. The
    /// equivalence oracle's input: serving never holds the history.
    #[cfg(any(test, feature = "oracle"))]
    pub fn records(&self, source: &dyn CommitSource, rev: &str) -> io::Result<Vec<CommitRecord>> {
        self.writable()?;
        let seg = |e: seg::SegError| io::Error::other(e.to_string());
        let order = source.rev_list(rev)?;
        let Some(store) = seg::read::Store::open(&self.dir).map_err(seg)? else {
            return Ok(Vec::new());
        };
        let rows = feed::DiskSource::open(store, &mut |p| Ok(p.to_vec())).map_err(seg)?;
        let mut records = Vec::with_capacity(order.len());
        for id in &order {
            if let Some(oid) = store::oid_of(id)
                && let Some(r) = rows.record_by_oid(&oid).map_err(seg)?
            {
                records.push(r);
            }
        }
        map_identities(source, rev, &mut records)?;
        let latest = wall_clock().saturating_add(insights::DAY);
        for r in &mut records {
            r.time = plausible_time(r.time, latest);
        }
        Ok(records)
    }
}

/// Of `order`, the ids the set does not hold, once each.
fn todo<'a>(order: &'a [String], held: &HashSet<seg::Oid>) -> Vec<&'a String> {
    let mut seen = HashSet::new();
    order
        .iter()
        .filter(|id| {
            !store::oid_of(id).is_some_and(|o| held.contains(&o)) && seen.insert(id.as_str())
        })
        .collect()
}

/// Tells the user that `n` commits could not be read and stay pending.
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
fn skipped(n: usize, dir: &std::path::Path) {
    eprintln!(
        "gitterrace: {n} commits could not be read and stay pending in {}",
        dir.display()
    );
}

/// Seconds since the epoch on this machine's clock.
fn wall_clock() -> i64 {
    // The equivalence oracle freezes the clock so period windows do not move between runs.
    if let Some(t) = std::env::var("GITTERRACE_TEST_NOW")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return t;
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// An author time every chart can place: from 1970 to `latest` (the wall clock plus a day).
///
/// Commit dates are free text from whoever wrote the commit; one dated in year 950 million or
/// in 2099 would otherwise size every time axis and end every period window. Such a commit is
/// still counted and listed, dated at the nearest edge. The wall clock is the bound because a
/// rebased tip can carry a future committer date too.
pub fn plausible_time(time: i64, latest: i64) -> i64 {
    time.clamp(0, latest.max(0))
}

/// "Now" for period windows over records, the oracle's twin of [`feed::reference_time`]: the
/// newest author time not after this machine's clock.
///
/// So an old repository still shows its last active stretch and a commit dated in the future (one
/// [`plausible_time`] pinned to the edge, or a skewed clock) never empties every window. Every
/// surface that cuts a period uses this one value.
#[cfg(any(test, feature = "oracle"))]
pub fn reference_time(records: &[CommitRecord]) -> i64 {
    reference_time_at(records, &feed::SystemClock)
}

/// [`reference_time`] with the clock given.
#[cfg(any(test, feature = "oracle"))]
pub fn reference_time_at(records: &[CommitRecord], clock: &dyn feed::Clock) -> i64 {
    let wall = clock.now();
    records
        .iter()
        .map(|r| r.time)
        .filter(|t| *t <= wall)
        .max()
        .unwrap_or(0)
}

/// Rewrites every record's author through the source's identity map, asking once per
/// distinct pair.
#[cfg(any(test, feature = "oracle"))]
fn map_identities(
    source: &dyn CommitSource,
    rev: &str,
    records: &mut [CommitRecord],
) -> io::Result<()> {
    let mut slot: HashMap<(String, String), usize> = HashMap::new();
    let mut pairs = Vec::new();
    for r in records.iter() {
        let key = (r.author.clone(), r.email.clone());
        if !slot.contains_key(&key) {
            slot.insert(key.clone(), pairs.len());
            pairs.push(key);
        }
    }
    let mapped = source.map_identities(rev, &pairs)?;
    if mapped.len() != pairs.len() {
        return Err(io::Error::other(
            "identity map returned the wrong number of pairs",
        ));
    }
    for r in records.iter_mut() {
        let key = (std::mem::take(&mut r.author), std::mem::take(&mut r.email));
        let (name, email) = &mapped[slot[&key]];
        r.author.clone_from(name);
        r.email.clone_from(email);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
