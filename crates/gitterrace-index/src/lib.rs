//! Incremental insights cache in `.git/gitterrace/`. One record per commit, appended;
//! statistics are computed from the records on demand by [`Insights`].

mod cache;
mod conventional;
pub mod feed;
mod insights;
pub mod seg;
mod source;
mod store;

use std::collections::{HashMap, HashSet};
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
pub const CACHE_FILES: [(&str, &str); 6] = [
    (
        store::VERSION_FILE,
        "The cache format. A mismatch deletes the cache and rebuilds it. The format is never migrated.",
    ),
    (
        store::COMMITS_FILE,
        "One record per commit, appended as the history is walked, with authors as written: the .mailmap of the revision shown is applied on read. Unreachable records are ignored, so rewritten history needs no cleanup, and a torn last line is dropped. A commit that cannot be read is not stored, so the next update retries it.",
    ),
    (
        store::PATHS_FILE,
        "Every file path any record mentions, one JSON string per line. Records refer to a path by its line number, so a path is stored once however many commits touch it.",
    ),
    (
        store::LOCK_FILE,
        "Empty. Held locked by whichever process is adding records, so two processes on one repository take turns instead of mixing up path numbers.",
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

/// The first store error `set` met while serving, from its tables or its rows. Some means the
/// cache is unreadable: [`Index::rebuild`] it, never serve part of it.
pub fn unreadable(set: &DiskSet) -> Option<String> {
    set.unreadable().or_else(|| set.base().unreadable())
}

/// The cache for one repository.
pub struct Index {
    dir: PathBuf,
    /// Threads that parse the stored records; 0 means one per core, at most 8.
    threads: usize,
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
            threads: 0,
            newer,
        })
    }

    /// Sets the threads that parse stored records; 0 means one per core, at most 8.
    #[must_use]
    pub const fn with_threads(mut self, threads: usize) -> Self {
        self.threads = threads;
        self
    }

    /// The cache format found on disk when a newer build wrote it. Then [`Index::update`] and
    /// [`Index::records`] fail with an explanation and leave the files alone.
    pub fn newer_format(&self) -> Option<&str> {
        self.newer.as_deref()
    }

    fn writable(&self) -> io::Result<()> {
        store::check_writable(&self.dir)
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    /// Size in bytes of the cache file. It changes whenever records are added, so a cache of
    /// parsed records can key on it.
    pub fn generation(&self) -> u64 {
        store::file_len(&self.dir)
    }

    /// Indexes commits reachable from `rev` that are not yet cached, and rebuilds those stored
    /// at a shallow boundary that has since been fetched. Waits for any other process doing the
    /// same; a commit the source cannot read is skipped and stays pending.
    pub fn update(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        progress: &dyn Fn(Progress),
    ) -> io::Result<()> {
        self.writable()?;
        let tip = source.resolve(rev);
        let ids = source.rev_list(rev)?;
        let shallow_list = source.shallow_ids();
        // A deepened or grafted clone changes topo ranks: same rule as an unreadable cache.
        store::check_shallow(&self.dir, &shallow_list)?;
        let shallow: HashSet<String> = shallow_list.iter().cloned().collect();
        // Everything below reads what another writer may be appending: lock first.
        let _lock = store::WriteLock::take(&self.dir)?;
        // Gone before the set can change, written back once it holds exactly `tip`: a crash in
        // between leaves no marker, so the next open checks the set the slow way.
        cache::remove(&self.dir, TIP_FILE)?;
        let stored = store::stored_ids(&self.dir)?;
        let todo = stored.todo(&ids, &shallow);
        let total = todo.len();
        progress(Progress { done: 0, total });
        let mut out = store::Appender::open(&self.dir, stored.valid)?;
        let order = ids.clone();
        let ids: Vec<String> = todo.into_iter().cloned().collect();
        // A first build gets every commit of `order`, in order, from the stream: fold each into
        // the segment as it passes, so the build never holds the whole history.
        let fresh = ids.len() == order.len()
            && seg::manifest::Manifest::load(&self.dir)
                .ok()
                .flatten()
                .is_none()
            && order.iter().all(|id| store::oid_of(id).is_some());
        let seg = |e: seg::SegError| io::Error::other(e.to_string());
        let mut build = if fresh {
            Some(store::SegmentBuild::new(&self.dir, None, &shallow_list).map_err(seg)?)
        } else {
            None
        };
        let mut stored = 0_usize;
        source.stream_records(
            &ids,
            BATCH,
            &mut |record| {
                stored += 1;
                if let Some(b) = build.as_mut() {
                    b.push(&record).map_err(seg)?;
                }
                out.push(&record, shallow.contains(&record.id))
            },
            &mut |done| progress(Progress { done, total }),
        )?;
        out.flush()?;
        // A skipped commit is retried later and then appended out of order, so the file is no
        // longer the reference bytes: say so, or a mismatch has no trace.
        if stored < ids.len() {
            skipped(ids.len() - stored, &self.dir);
        }
        let complete = match build {
            // Complete: every commit of the rev is in the segment.
            Some(b) if stored == ids.len() => {
                b.finish(None, Vec::new()).map_err(seg)?;
                true
            }
            // A skipped commit leaves a gap: the next update builds once it is readable.
            Some(_) => false,
            None => {
                self.build_segments(&order, &shallow_list)?;
                stored == ids.len()
            }
        };
        if complete && let Some(t) = tip {
            cache::write_atomic(&self.dir, TIP_FILE, t.as_bytes())?;
        }
        Ok(())
    }

    /// Writes the segment set for `order` (canonical ids of the rev) when the manifest does not
    /// already hold exactly that order. Runs under the writer lock. The records are read once
    /// here: a peak while building is accepted, serving never holds them.
    fn build_segments(&self, order: &[String], shallow: &[String]) -> io::Result<()> {
        let seg = |e: seg::SegError| io::Error::other(e.to_string());
        let previous = seg::manifest::Manifest::load(&self.dir).ok().flatten();
        // The set already holds exactly this history (merged or not): nothing to write, and
        // no record parse on a no-op update.
        let held = seg::read::Store::open(&self.dir)
            .ok()
            .flatten()
            .and_then(|s| store::load_order(&s).ok());
        if let Some(h) = &held
            && h.len() == order.len()
        {
            let h: HashSet<&seg::Oid> = h.iter().collect();
            if order
                .iter()
                .all(|id| store::oid_of(id).is_some_and(|o| h.contains(&o)))
            {
                return Ok(());
            }
        }
        let (records, _) = store::read_records(&self.dir, self.threads)?;
        let records = store::select(records, order);
        let oids: Vec<seg::Oid> = records
            .iter()
            .filter_map(|r| store::oid_of(&r.id))
            .collect();
        // A source whose ids are not oids (a test double) has no order file to write.
        if oids.len() != records.len() {
            return Ok(());
        }
        // Incremental when every commit the set holds is still reachable: the new ones become
        // one more segment. Otherwise (first build, rewritten history) a fresh build.
        if let Some(prev) = previous.as_ref()
            && let Some(old) = held
            && old.len() == prev.commits as usize
            && old.len() < oids.len()
        {
            let old: HashSet<seg::Oid> = old.into_iter().collect();
            let reach: HashSet<&seg::Oid> = oids.iter().collect();
            if old.iter().all(|o| reach.contains(o)) {
                let fresh = records
                    .iter()
                    .zip(&oids)
                    .filter(|(_, o)| !old.contains(*o))
                    .map(|(r, _)| r);
                let after =
                    store::append_segment(&self.dir, prev.clone(), shallow, fresh).map_err(seg)?;
                drop(records);
                let segments = after
                    .live
                    .iter()
                    .filter(|f| f.kind == seg::format::Kind::Segment as u8)
                    .count();
                if segments > store::MAX_SEGMENTS {
                    self.merge_in_background(order, shallow);
                }
                return Ok(());
            }
        }
        let mut build =
            store::SegmentBuild::new(&self.dir, previous.as_ref(), shallow).map_err(seg)?;
        for r in &records {
            build.push(r).map_err(seg)?;
        }
        drop(records);
        build.finish(previous, Vec::new()).map_err(seg)?;
        Ok(())
    }

    /// Folds the segments into one once there are more than 8, off the caller's thread, so a
    /// request never waits on it. The merge takes the writer lock itself.
    #[expect(
        clippy::print_stderr,
        reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
    )]
    fn merge_in_background(&self, order: &[String], shallow: &[String]) {
        let (dir, threads) = (self.dir.clone(), self.threads);
        let (order, shallow) = (order.to_vec(), shallow.to_vec());
        let run = move || -> io::Result<bool> {
            // Until the disk base lands a record comes from today's record file.
            let (records, _) = store::read_records(&dir, threads)?;
            let mut by_id: HashMap<String, CommitRecord> =
                records.into_iter().map(|r| (r.id.clone(), r)).collect();
            store::merge_if_needed(&dir, &order, &shallow, Vec::new(), &mut |id| {
                by_id
                    .remove(id)
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

    /// Commits reachable from `rev` that [`Index::update`] would build. Zero means
    /// [`Index::records`] for `rev` is complete; insights for a branch other than HEAD call
    /// [`Index::update`] first.
    pub fn pending(&self, source: &dyn CommitSource, rev: &str) -> io::Result<usize> {
        let ids = source.rev_list(rev)?;
        let shallow: HashSet<String> = source.shallow_ids().into_iter().collect();
        Ok(store::stored_ids(&self.dir)?.todo(&ids, &shallow).len())
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
    ) -> io::Result<Option<std::sync::Arc<DiskSet>>> {
        let seg = |e: seg::SegError| io::Error::other(e.to_string());
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
            .map_err(seg)?;
        let Some(tables) = open()? else {
            return Ok(None);
        };
        Ok(Some(std::sync::Arc::new(feed::SegmentSource::new(
            disk,
            tables.segments,
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
    /// up to [`feed::FRESH_CAP`] it lacks, read from git. `tip` is `rev` resolved to a commit
    /// id. Past the cap the answer is [`feed::RevView::NeedsReindex`]: building the set for
    /// this commit would move it off HEAD, and holding its records would break the memory
    /// bound. `None` when there is no set.
    pub fn rev_source(
        &self,
        source: &dyn CommitSource,
        rev: &str,
        tip: &str,
    ) -> io::Result<Option<feed::RevView<DiskSet>>> {
        let seg = |e: seg::SegError| io::Error::other(e.to_string());
        let open = || seg::read::Store::open(&self.dir).map_err(seg);
        let (Some(rows), Some(tables)) = (open()?, open()?) else {
            return Ok(None);
        };
        let disk = feed::DiskSource::open(rows, &mut |pairs| source.map_identities(rev, pairs))
            .map_err(seg)?;
        let set = feed::SegmentSource::new(disk, tables.segments);
        feed::rev_view(set, source, rev, tip, feed::FRESH_CAP).map(Some)
    }

    /// The memory adapter over `records` (from [`Index::records`]), for a rev the segment set
    /// does not hold. Holds that history while used.
    pub fn memory_source(
        records: std::sync::Arc<Vec<CommitRecord>>,
    ) -> std::sync::Arc<dyn feed::InsightsSource> {
        std::sync::Arc::new(feed::MemorySource::shared(records))
    }

    /// All cached records reachable from `rev`, oldest first, with authors mapped through the
    /// `.mailmap` at the tip of `rev`.
    pub fn records(&self, source: &dyn CommitSource, rev: &str) -> io::Result<Vec<CommitRecord>> {
        let order = source.rev_list(rev)?;
        let (records, _) = store::read_records(&self.dir, self.threads)?;
        let mut records = store::select(records, &order);
        map_identities(source, rev, &mut records)?;
        let latest = wall_clock().saturating_add(insights::DAY);
        for r in &mut records {
            r.time = plausible_time(r.time, latest);
        }
        Ok(records)
    }
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

/// "Now" for period windows: the newest author time not after this machine's clock.
///
/// So an old repository still shows its last active stretch and a commit dated in the future (one
/// [`plausible_time`] pinned to the edge, or a skewed clock) never empties every window. Every
/// surface that cuts a period uses this one value.
pub fn reference_time(records: &[CommitRecord]) -> i64 {
    let wall = wall_clock();
    records
        .iter()
        .map(|r| r.time)
        .filter(|t| *t <= wall)
        .max()
        .unwrap_or(0)
}

/// Rewrites every record's author through the source's identity map, asking once per
/// distinct pair.
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
