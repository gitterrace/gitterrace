mod segments;

use std::fs::File;
use std::io;
use std::path::Path;

use crate::cache;

pub(crate) use segments::{
    MAX_SEGMENTS, SegmentBuild, append_segment, check_shallow, load_order, merge_if_needed, oid_of,
    shallow_stale,
};

/// Cache format; a different one on disk discards the cache.
pub const FORMAT_VERSION: &str = "7";
pub(crate) const VERSION_FILE: &str = "VERSION";
pub(crate) const LOCK_FILE: &str = "lock";

/// Why a cache cannot be written by this build.
pub(crate) fn newer_message(found: &str) -> String {
    format!(
        "the cache in this repository has format {found}, newer than this build's \
         {FORMAT_VERSION}; \
         leaving it untouched. Run the newer gitterrace, or delete .git/gitterrace to rebuild"
    )
}

/// The stored format when it is newer than ours. Older, missing and unreadable versions are
/// ours to reset; a newer one belongs to a newer build and must never be overwritten.
pub(crate) fn newer_version(dir: &Path) -> io::Result<Option<String>> {
    let found = cache::read(dir, VERSION_FILE)?
        .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
        .unwrap_or_default();
    let newer = match (found.parse::<u64>(), FORMAT_VERSION.parse::<u64>()) {
        (Ok(f), Ok(ours)) => f > ours,
        _ => false,
    };
    Ok(newer.then_some(found))
}

/// Fails when the cache now belongs to a newer build. Called right before every write, so a
/// long-running older process cannot append rows in its format to a rewritten cache.
pub(crate) fn check_writable(dir: &Path) -> io::Result<()> {
    newer_version(dir)?.map_or(Ok(()), |found| Err(io::Error::other(newer_message(&found))))
}

/// Creates `dir` and resets it when its format version differs from ours. A newer version is
/// left alone and returned, so the caller can refuse to use it.
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
pub(crate) fn ensure_dir(dir: &Path) -> io::Result<Option<String>> {
    cache::cache_dir(dir)?;
    if let Some(found) = newer_version(dir)? {
        eprintln!("gitterrace: {}", newer_message(&found));
        return Ok(Some(found));
    }
    if needs_reset(dir)? {
        // Another process may be appending: deleting its files under it would tear them. The
        // check is repeated under the lock, since the writer that held it may have reset too.
        // Only a reset takes the lock, so a read-only cache that is current stays readable.
        let _lock = WriteLock::take(dir)?;
        if version_stale(dir)? || manifest_unreadable(dir) {
            wipe(dir)?;
        } else {
            // A crash's temp file or a retired file: not part of the live set, so it goes alone.
            sweep(dir, &known_files(dir))?;
        }
    }
    Ok(None)
}

/// Deletes every file in the cache except the lock and writes our `VERSION`. A cache this build
/// cannot read is never partly used: whatever it held, old formats, stray or half-written files,
/// goes. The caller holds the lock and has checked the cache is not a newer build's.
pub(crate) fn wipe(dir: &Path) -> io::Result<()> {
    sweep(dir, &[LOCK_FILE.to_owned()])?;
    cache::write_atomic(dir, VERSION_FILE, format!("{FORMAT_VERSION}\n").as_bytes())
}

/// Removes every entry of `dir` not named in `live`, the retired-file sweep. Links and
/// special files go through `cache::remove`, which never follows them. A file that cannot be
/// removed now (open elsewhere, on Windows) is left for the next sweep, so this never fails on
/// it. Directories are left alone: the cache never makes one.
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
pub(crate) fn sweep(dir: &Path, live: &[String]) -> io::Result<()> {
    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !live.iter().any(|l| l == name) && !in_flight(dir, name) {
            names.push(name.to_owned());
        }
    }
    names.sort();
    for name in names {
        if std::fs::symlink_metadata(dir.join(&name)).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        if let Err(e) = cache::remove(dir, &name) {
            eprintln!("gitterrace: could not remove {name} from the cache, retrying later: {e}");
        }
    }
    Ok(())
}

/// Whether the stored format differs from ours.
fn version_stale(dir: &Path) -> io::Result<bool> {
    let current = cache::read(dir, VERSION_FILE)?;
    let current = current
        .as_deref()
        .map(|b| String::from_utf8_lossy(b).trim().to_owned());
    Ok(current.as_deref() != Some(FORMAT_VERSION))
}

/// Whether `ensure_dir` has files to delete or sweep: a stale format, an unreadable set, or a
/// file this format does not name.
fn needs_reset(dir: &Path) -> io::Result<bool> {
    Ok(version_stale(dir)? || manifest_unreadable(dir) || has_foreign(dir)?)
}

/// Files of this format: the documented registry plus `MANIFEST` and every file it names, so
/// a newly registered file is live without a second list to forget. Anything else is another
/// format's or a crash's leftover. A manifest that does not decode names nothing: its files
/// are then strays and the manifest itself is not live, so the cache is wiped and rebuilt.
fn known_files(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = crate::CACHE_FILES
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    if let Ok(Some(m)) = crate::seg::manifest::Manifest::load(dir) {
        out.push(crate::seg::manifest::MANIFEST.to_owned());
        out.extend(m.live.into_iter().map(|f| f.name));
    }
    out
}

/// A temp file younger than this may be another process's write in progress: `lines.json`
/// (insights/headfiles.rs) is written outside the lock, so its temp file can appear while a
/// writer sweeps under the lock. An older one is a crash's leftover and goes.
const TEMP_GRACE: std::time::Duration = std::time::Duration::from_secs(3600);

fn in_flight(dir: &Path, name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("tmp"))
        && std::fs::symlink_metadata(dir.join(name))
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().map_or(true, |age| age < TEMP_GRACE))
}

/// A `MANIFEST` that does not decode or names a file that is missing or differs from it: the
/// set it describes cannot be served whole, so the cache is wiped and rebuilt.
fn manifest_unreadable(dir: &Path) -> bool {
    match crate::seg::manifest::Manifest::load(dir) {
        Ok(None) => false,
        Ok(Some(m)) => m.verify(dir).is_err(),
        Err(_) => true,
    }
}

fn has_foreign(dir: &Path) -> io::Result<bool> {
    let known = known_files(dir);
    for entry in std::fs::read_dir(dir)? {
        let name = entry?.file_name();
        if !name
            .to_str()
            .is_some_and(|n| known.iter().any(|k| k == n) || in_flight(dir, n))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Holds the cache's writer lock until dropped. Every writer, in this process or another
/// (`serve` beside `mcp`), takes it before reading the set it extends, so two writers never build
/// on one manifest.
pub(crate) struct WriteLock(File);

impl WriteLock {
    /// Blocks until no other writer holds the lock.
    pub(crate) fn take(dir: &Path) -> io::Result<Self> {
        let file = cache::open_append(dir, LOCK_FILE)?;
        file.lock()?;
        Ok(Self(file))
    }
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        // Closing the handle releases the lock too; unlocking first only makes it prompt.
        drop(self.0.unlock());
    }
}

#[cfg(test)]
mod tests;
