//! Every path under `.git/gitterrace/` goes through here. A repository can ship that
//! directory (a tarball, a zip), so any entry may be a planted symlink: following one would
//! read or overwrite a file outside the repository. Links and non-regular entries are removed
//! (removing a link never touches its target) and the cache is rebuilt; writes open with
//! `O_NOFOLLOW` so a link planted between the check and the open still fails.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Makes `dir` a real directory, replacing a symlink or other non-directory entry.
pub(crate) fn cache_dir(dir: &Path) -> io::Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => return Ok(()),
        Ok(_) => {
            eprintln!(
                "gitterrace: {} is not a directory, replacing it with a fresh cache",
                dir.display()
            );
            fs::remove_file(dir)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if let Some(parent) = dir.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::create_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            // Lost a race: accept only a real directory.
            if fs::symlink_metadata(dir)?.is_dir() {
                Ok(())
            } else {
                Err(refused(dir))
            }
        }
        r => r,
    }
}

/// Path of `name` in the cache `dir`, after checking that neither is a link and that the
/// file, when present, is a regular file. A link or special file is removed so the caller
/// sees it as missing.
pub(crate) fn cache_file(dir: &Path, name: &str) -> io::Result<PathBuf> {
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err(refused(dir));
    }
    let path = dir.join(name);
    match fs::symlink_metadata(&path) {
        Ok(m) if m.is_file() => {}
        Ok(m) if m.is_dir() => return Err(refused(&path)),
        Ok(_) => {
            eprintln!(
                "gitterrace: {} is not a regular file, discarding it",
                path.display()
            );
            fs::remove_file(&path)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(path)
}

/// Reads a cache file; a missing file reads as `None`.
pub(crate) fn read(dir: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    let path = cache_file(dir, name)?;
    match no_follow(OpenOptions::new().read(true)).open(path) {
        Ok(mut f) => {
            let mut out = Vec::new();
            io::Read::read_to_end(&mut f, &mut out)?;
            Ok(Some(out))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Opens a cache file for appending, creating it.
pub(crate) fn open_append(dir: &Path, name: &str) -> io::Result<File> {
    let path = cache_file(dir, name)?;
    no_follow(OpenOptions::new().create(true).append(true).read(true)).open(path)
}

/// Replaces a cache file atomically: a fresh, uniquely named temp file, then a rename. The
/// rename replaces a link planted at `name` instead of writing through it.
pub(crate) fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let path = cache_file(dir, name)?;
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = no_follow(OpenOptions::new().write(true).create_new(true))
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes))
        .and_then(|()| fs::rename(&tmp, &path));
    if written.is_err() {
        // Best effort: the temp name is ours alone and only clutters the cache.
        drop(fs::remove_file(&tmp));
    }
    written
}

/// Removes a cache file; a missing one is fine.
pub(crate) fn remove(dir: &Path, name: &str) -> io::Result<()> {
    match fs::remove_file(cache_file(dir, name)?) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Length of a cache file, 0 when missing or refused.
pub(crate) fn len(dir: &Path, name: &str) -> u64 {
    cache_file(dir, name)
        .and_then(fs::symlink_metadata)
        .map_or(0, |m| m.len())
}

#[cfg(unix)]
fn no_follow(o: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    o.custom_flags(libc::O_NOFOLLOW)
}

#[cfg(not(unix))]
fn no_follow(o: &mut OpenOptions) -> &mut OpenOptions {
    o
}

fn refused(path: &Path) -> io::Error {
    io::Error::other(format!(
        "refusing cache path {}: not a regular entry",
        path.display()
    ))
}
