//! Incremental insights cache in `.git/gitterrace/`: the segment set, appended per update and
//! merged in the background. Statistics read it through [`feed::InsightsSource`] on demand,
//! by [`Insights`].

mod cache;
mod chain;
mod conventional;
pub mod feed;
pub mod graph;
mod insights;
pub mod messages;
#[cfg(any(test, feature = "oracle"))]
mod oracle;
mod phases;
pub mod query;
pub mod seg;
mod select;
mod source;
mod store;
mod tip;

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use gitterrace_git::Repo;

/// Re-exported so the cards crate shares the one date rules without a new dependency.
pub use conventional::parse_subject;
pub use gitterrace_git::{civil_from_days, days_from_civil};
pub use insights::*;
#[cfg(any(test, feature = "oracle"))]
pub use oracle::{reference_time, reference_time_at};
pub use phases::PhaseTime;
pub use select::Selected;
pub use source::CommitSource;
pub use store::FORMAT_VERSION;

/// Every file the cache keeps in `.git/gitterrace/`, with what it holds. The architecture page is
/// rendered from this, and a test fails when the folder holds a file not listed here.
pub const CACHE_FILES: [(&str, &str); 5] = [
    (
        store::VERSION_FILE,
        "The cache format, `v` and the number. A mismatch deletes the cache and rebuilds it. The \
        format is never migrated.",
    ),
    (
        store::LOCK_FILE,
        "Empty. Held locked by whichever process is extending the segment set, so two processes on \
        one repository take turns instead of building on one manifest.",
    ),
    (
        store::OWNER_FILE,
        "Only in a cache outside the repository: the real path of the git dir it belongs to. \
        Another path there wipes the cache, so a hash collision rebuilds.",
    ),
    (
        TIP_FILE,
        "The commit id the segment files were last built for. Opening insights for that commit \
        skips walking its history to check the files hold exactly it.",
    ),
    (
        insights::CACHE_FILE,
        "Line counts per blob id for the language chart. A blob never changes, so the counts never \
        go stale.",
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
    /// Whether updates keep the message column, see [`Index::set_message_index`].
    messages: std::sync::atomic::AtomicBool,
    /// Wall time per step of the last update, see [`Index::phases`].
    phases: phases::Phases,
}

impl Index {
    /// Opens or creates the cache under `repo.git_dir()/gitterrace`, or in the folder
    /// `store::fallback_dir` names when that cannot be written.
    pub fn open(repo: &Repo) -> io::Result<Self> {
        let git = repo.git_dir();
        Self::open_with(git.join("gitterrace"), store::fallback_dir(git))
    }

    /// [`Index::open`] with the user-cache folder passed in. Without one: [`Index::open_dir`].
    fn open_with(own: PathBuf, fallback: Option<(PathBuf, PathBuf)>) -> io::Result<Self> {
        let Some((dir, real)) = fallback else {
            return Self::open_dir(own);
        };
        match Self::open_dir(own.clone()).and_then(|i| store::writable(&own).map(|()| i)) {
            Ok(i) => Ok(i),
            Err(e) if store::cannot_write(&e) => {
                let index = Self::open_dir(dir)?;
                if index.newer.is_none() {
                    store::claim(&index.dir, &real)?;
                }
                Ok(index)
            }
            Err(e) => Err(e),
        }
    }

    /// Opens or creates the cache in `dir`. A format version mismatch discards old records.
    pub fn open_dir(dir: PathBuf) -> io::Result<Self> {
        let newer = store::ensure_dir(&dir)?;
        Ok(Self {
            dir,
            fresh_cap: feed::FRESH_CAP,
            block_cache: std::sync::atomic::AtomicUsize::new(feed::block_cache_default()),
            newer,
            messages: std::sync::atomic::AtomicBool::new(false),
            phases: phases::Phases::default(),
        })
    }

    /// Whether updates keep the message column commit search reads (`index.has_messages`).
    /// On, the next update builds it once and extends it with every later update. Off, the next
    /// update drops it and commit search walks git.
    pub fn set_message_index(&self, on: bool) {
        self.messages
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// The setting [`Index::set_message_index`] put in force.
    pub fn message_index(&self) -> bool {
        self.messages.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The commits matching `query` reachable from `tip` (a full commit id), newest first, at
    /// most `most`, read from the message column. `None` when the column is off, not built yet
    /// or built for another commit: the caller walks git then. An unreadable column is an
    /// error [`feed::SourceError::of`] finds, which callers turn into a rebuild.
    pub fn search_messages(
        &self,
        tip: &str,
        query: &messages::MessageQuery<'_>,
        most: usize,
    ) -> io::Result<Option<messages::MessageHits>> {
        if !self.message_index() || self.newer.is_some() {
            return Ok(None);
        }
        let seg = seg_io;
        let Some(store) = seg::read::Store::open(&self.dir).map_err(seg)? else {
            return Ok(None);
        };
        let Some(col) = messages::MessageColumn::of(store).map_err(seg)? else {
            return Ok(None);
        };
        col.search(tip, query, most).map_err(seg)
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

    /// The format a newer build wrote the cache in, read from disk now: a newer build may take
    /// the cache over while this one runs. Such a cache is never written by this build.
    pub fn newer_owner(&self) -> Option<String> {
        store::newer_version(&self.dir).ok().flatten()
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
        let ph = &self.phases;
        ph.reset();
        ph.start("rev_list");
        let tip = source.resolve(rev);
        let order = source.rev_list(rev)?;
        let shallow = source.shallow_ids();
        ph.start("held");
        // A deepened or grafted clone changes topo ranks: same rule as an unreadable cache.
        store::check_shallow(&self.dir, &shallow)?;
        // Everything below reads the set another writer may be extending: lock first.
        let _lock = store::WriteLock::take(&self.dir)?;
        // Gone before the set can change, written back once it holds exactly `tip`: a crash in
        // between leaves no marker, so the next open checks the set the slow way.
        cache::remove(&self.dir, TIP_FILE)?;
        let seg = seg_io;
        let previous = seg::manifest::Manifest::load(&self.dir).map_err(seg)?;
        let held = self.held()?;
        let todo = todo(&order, &held);
        if todo.is_empty() {
            ph.start("messages");
            self.sync_messages(source)?;
            ph.start("chain");
            chain::sync(&self.dir)?;
            tip::sync(&self.dir)?;
            ph.end();
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
        // The stream's wall time holds the git reads and the fold; the fold is timed apart.
        let mut fold = std::time::Duration::ZERO;
        ph.start("stream");
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
            ph.start("append");
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
                    let t = std::time::Instant::now();
                    let pushed = build.push(&r).map_err(seg);
                    fold += t.elapsed();
                    pushed
                },
                &mut |done| progress(Progress { done, total }),
            )?;
            ph.start("finish");
            build.finish(previous, Vec::new()).map_err(seg)?;
        }
        ph.add("stream.fold", fold);
        ph.start("messages");
        self.sync_messages(source)?;
        ph.start("chain");
        chain::sync(&self.dir)?;
        ph.start("tip");
        tip::sync(&self.dir)?;
        ph.end();
        // A skipped commit is not in the set, so it stays pending and the next update appends
        // it; until then HEAD's pages read the reach view, and the tip marker is not written.
        if read < total {
            skipped(total - read, &self.dir);
        } else if let Some(t) = tip {
            cache::write_atomic(&self.dir, TIP_FILE, t.as_bytes())?;
        }
        Ok(())
    }

    /// Builds or extends the message column when it is on, drops it when it is off. Under the
    /// writer lock the caller holds.
    fn sync_messages(&self, source: &dyn CommitSource) -> io::Result<()> {
        if self.message_index() {
            return messages::extend(&self.dir, source);
        }
        let Some(mut m) = seg::manifest::Manifest::load(&self.dir).map_err(seg_io)? else {
            return Ok(());
        };
        let msg = seg::format::Kind::Messages as u8;
        if !m.live.iter().any(|f| f.kind == msg) {
            return Ok(());
        }
        let (gone, kept): (Vec<_>, Vec<_>) = m.live.into_iter().partition(|f| f.kind == msg);
        m.live = kept;
        m.retired.extend(gone.into_iter().map(|f| f.name));
        m.commit(&self.dir).map_err(seg_io)
    }

    /// Wall time per step of the update running now or last finished, in order, the running
    /// one with its time so far, and its name. `stream.fold` is the part of `stream` spent
    /// folding records into the segment, the rest is reading and diffing git.
    pub fn phases(&self) -> (Vec<PhaseTime>, Option<&'static str>) {
        self.phases.snapshot()
    }

    /// The commits the segment set holds, empty without a set.
    fn held(&self) -> io::Result<HashSet<seg::Oid>> {
        let seg = seg_io;
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
        let run = move || store::merge_stored(&dir, &order, &shallow);
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

    /// Deletes the whole cache (an unreadable one is never repaired in place), so the next update
    /// rebuilds it. A newer build's cache is never touched.
    pub fn rebuild(&self) -> io::Result<()> {
        self.writable()?;
        let _lock = store::WriteLock::take(&self.dir)?;
        store::wipe(&self.dir)
    }

    /// Every commit reachable from `rev` as a record, read back from the segment set, oldest
    /// first, with the identities of `rev`'s `.mailmap` and times pinned as pages see them. The
    /// equivalence oracle's input: serving never holds the history.
    #[cfg(any(test, feature = "oracle"))]
    pub fn records(&self, source: &dyn CommitSource, rev: &str) -> io::Result<Vec<CommitRecord>> {
        self.writable()?;
        let seg = seg_io;
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
        oracle::map_identities(source, rev, &mut records)?;
        let latest = wall_clock().saturating_add(insights::DAY);
        for r in &mut records {
            r.time = plausible_time(r.time, latest);
        }
        Ok(records)
    }
}

/// A store error as an `io::Error`. An unreadable file stays a [`feed::SourceError`], so a
/// caller of [`Index::update`] can tell "rebuild" from any other failure.
pub(crate) fn seg_io(e: seg::SegError) -> io::Error {
    match e {
        seg::SegError::Unreadable { .. } => feed::SourceError::io(e),
        e => io::Error::other(e.to_string()),
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

/// Seconds since the epoch on this machine's clock, frozen by `GITTERRACE_TEST_NOW`. The one
/// reader of the clock: the web pages and the registry stamp through it.
pub fn wall_clock() -> i64 {
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

#[cfg(test)]
mod tests;
