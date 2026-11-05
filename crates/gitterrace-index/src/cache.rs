//! Every path under `.git/gitterrace/` goes through here. A repository can ship that
//! directory (a tarball, a zip), so any entry may be a planted symlink: following one would
//! read or overwrite a file outside the repository. Links and non-regular entries are removed
//! (removing a link never touches its target) and the cache is rebuilt; writes open with
//! `O_NOFOLLOW` so a link planted between the check and the open still fails.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

/// Makes `dir` a real directory, replacing a symlink or other non-directory entry.
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
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
#[expect(
    clippy::print_stderr,
    reason = "the index crate has no logging facade: a cache warning goes to the user's stderr"
)]
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
    cache_file(dir, name)?;
    gitterrace_git::read_no_follow(dir, name)
}

/// Opens a cache file for appending, creating it.
pub(crate) fn open_append(dir: &Path, name: &str) -> io::Result<File> {
    cache_file(dir, name)?;
    gitterrace_git::open_append_no_follow(dir, name, 0o644)
}

/// Replaces a cache file atomically through the shared no-follow writer.
pub(crate) fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    cache_file(dir, name)?;
    gitterrace_git::write_atomic(dir, name, bytes, 0o644)
}

/// Removes a cache file; a missing one is fine.
pub(crate) fn remove(dir: &Path, name: &str) -> io::Result<()> {
    match fs::remove_file(cache_file(dir, name)?) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

fn refused(path: &Path) -> io::Error {
    io::Error::other(format!(
        "refusing cache path {}: not a regular entry",
        path.display()
    ))
}
