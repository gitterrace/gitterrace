//! Every path under `.git/gitterrace/` goes through here. A repository can ship that
//! directory (a tarball, a zip), so any entry may be a planted symlink: following one would
//! read or overwrite a file outside the repository. Links and non-regular entries are removed
//! (removing a link never touches its target) and the cache is rebuilt; writes open with
//! `O_NOFOLLOW` (unix) or `FILE_FLAG_OPEN_REPARSE_POINT` (Windows) so a link planted between
//! the check and the open still fails.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use gitterrace_git::refused;

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
    // A cache inside a repository's `.git` is made again alone, never with its parent: a
    // `.git` that is gone (the repository deleted under a running server) made again here
    // would bring the folder back as a repository, served forever. A cache folder outside a
    // repository (the one used when `.git` is read-only) gets its parents.
    if let Some(parent) = dir.parent() {
        if parent.file_name() == Some(std::ffi::OsStr::new(".git")) {
            if !parent.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} is gone", parent.display()),
                ));
            }
        } else {
            fs::create_dir_all(parent)?;
        }
    }
    match fs::create_dir(dir) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            // Lost a race: accept only a real directory.
            if fs::symlink_metadata(dir)?.is_dir() { Ok(()) } else { Err(refused(dir)) }
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
            eprintln!("gitterrace: {} is not a regular file, discarding it", path.display());
            fs::remove_file(&path)?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(path)
}

/// Longest stamp file (`VERSION`, the owner path, the built tip): a few bytes when sound.
/// Not configurable: a size bound on a file the repository may have planted.
pub(crate) const MAX_STAMP: u64 = 16 << 10;
/// Longest whole-read cache file (`lines.json`): far above any real head tree's line counts.
/// Not configurable: a size bound on a file the repository may have planted.
const MAX_READ: u64 = 64 << 20;

/// Reads a cache file of at most 64 MiB; a missing file reads as `None`.
pub(crate) fn read(dir: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    read_bounded(dir, name, MAX_READ)
}

/// Reads a stamp file of at most `MAX_STAMP` bytes; a missing file reads as `None`.
pub(crate) fn read_stamp(dir: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    read_bounded(dir, name, MAX_STAMP)
}

/// Reads a cache file without following a link, blocking on a FIFO or reading past `max`.
fn read_bounded(dir: &Path, name: &str, max: u64) -> io::Result<Option<Vec<u8>>> {
    cache_file(dir, name)?;
    gitterrace_git::read_no_follow_bounded(dir, name, max)
}

/// Opens a cache file for positional reads: no link followed, no FIFO blocked on, and the
/// opened handle a regular file. A missing file opens as `None`.
pub(crate) fn open_read(dir: &Path, name: &str) -> io::Result<Option<File>> {
    cache_file(dir, name)?;
    gitterrace_git::open_read_no_follow(dir, name)
}

pub(crate) fn open_append(dir: &Path, name: &str) -> io::Result<File> {
    cache_file(dir, name)?;
    gitterrace_git::open_append_no_follow(dir, name, 0o644)
}

/// Replaces a cache file atomically through the shared no-follow writer.
pub(crate) fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    cache_file(dir, name)?;
    gitterrace_git::write_atomic(dir, name, bytes, 0o644)
}

pub(crate) fn remove(dir: &Path, name: &str) -> io::Result<()> {
    match fs::remove_file(cache_file(dir, name)?) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
