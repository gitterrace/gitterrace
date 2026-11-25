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

/// Cache files are owner-only: checkpoints hold copies of uncommitted text and findings hold
/// review notes, so they must not inherit the umask. A shared repository therefore serves
/// each user from their own cache: another user cannot write `.git/gitterrace`, so they take
/// the user-cache folder, and a cache of the other user is never read.
pub(crate) const FILE_MODE: u32 = 0o600;
/// Cache folders, the parents of a user-cache folder included.
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;

/// Makes `dir` a real directory, replacing a symlink or other non-directory entry.
pub(crate) fn cache_dir(dir: &Path) -> io::Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(m) if m.is_dir() => return tighten(dir),
        Ok(_) => {
            crate::warn(format_args!(
                "{} is not a directory, replacing it with a fresh cache",
                dir.display()
            ));
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
            private_dirs().recursive(true).create(parent)?;
        }
    }
    match private_dirs().create(dir) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            // Lost a race: accept only a real directory.
            if fs::symlink_metadata(dir)?.is_dir() { tighten(dir) } else { Err(refused(dir)) }
        }
        r => r,
    }
}

fn private_dirs() -> fs::DirBuilder {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        let mut b = fs::DirBuilder::new();
        b.mode(DIR_MODE);
        b
    }
    #[cfg(not(unix))]
    fs::DirBuilder::new()
}

/// Migrates a cache made with the default mode: the folder and each regular file in it lose
/// group and other access, through no-follow handles so a planted link is left alone.
fn tighten(dir: &Path) -> io::Result<()> {
    gitterrace_git::make_private_no_follow(dir)?;
    for entry in fs::read_dir(dir)? {
        gitterrace_git::make_private_no_follow(&entry?.path())?;
    }
    Ok(())
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
            crate::warn(format_args!("{} is not a regular file, discarding it", path.display()));
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
    gitterrace_git::open_append_no_follow(dir, name, FILE_MODE)
}

/// Replaces a cache file atomically through the shared no-follow writer.
pub(crate) fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    cache_file(dir, name)?;
    gitterrace_git::write_atomic(dir, name, bytes, FILE_MODE)
}

pub(crate) fn remove(dir: &Path, name: &str) -> io::Result<()> {
    match fs::remove_file(cache_file(dir, name)?) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use super::*;

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).map_or(0, |m| m.permissions().mode() & 0o777)
    }

    /// A traversable repository: the umask of a normal login (022) never widens a 0600 or 0700
    /// request, so these modes hold whatever the umask is.
    fn repo() -> io::Result<(tempfile::TempDir, PathBuf)> {
        let tmp = tempfile::tempdir()?;
        let git = tmp.path().join(".git");
        fs::create_dir(&git)?;
        fs::set_permissions(&git, fs::Permissions::from_mode(0o755))?;
        let dir = git.join("gitterrace");
        cache_dir(&dir)?;
        Ok((tmp, dir))
    }

    /// Red when any cache entry a writer makes is group or world accessible. Also catches a
    /// stamp or lock file left at 0644, which the same writers produce.
    #[test]
    fn new_and_rewritten_entries_are_owner_only() -> io::Result<()> {
        let (_t, dir) = repo()?;
        assert_eq!(mode(&dir), 0o700);
        write_atomic(&dir, "viewed", b"one")?;
        assert_eq!(mode(&dir.join("viewed")), 0o600);
        write_atomic(&dir, "viewed", b"two")?;
        assert_eq!(mode(&dir.join("viewed")), 0o600);
        open_append(&dir, "checkpoints")?;
        assert_eq!(mode(&dir.join("checkpoints")), 0o600);
        crate::seg::io::write_atomic(&dir, "MANIFEST", b"m")?;
        assert_eq!(mode(&dir.join("MANIFEST")), 0o600);
        Ok(())
    }

    /// Red when a user-cache location (parents made on the way) keeps the default 0755.
    #[test]
    fn a_user_cache_dir_and_its_parents_are_owner_only() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("cache").join("index").join("abc");
        cache_dir(&dir)?;
        for p in [&dir, dir.parent().unwrap_or(&dir), &tmp.path().join("cache")] {
            assert_eq!(mode(p), 0o700, "{}", p.display());
        }
        Ok(())
    }

    /// Red when a cache made by an older build stays readable: dir, an appended file and a
    /// read-only one that nothing rewrites.
    #[test]
    fn an_existing_cache_is_tightened_when_opened() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let dir = tmp.path().join("gitterrace");
        fs::create_dir(&dir)?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755))?;
        for n in ["checkpoints", "seg-1-0.gts"] {
            fs::write(dir.join(n), "x")?;
            fs::set_permissions(dir.join(n), fs::Permissions::from_mode(FILE_MODE))?;
        }
        cache_dir(&dir)?;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join("checkpoints")), 0o600);
        assert_eq!(mode(&dir.join("seg-1-0.gts")), 0o600);
        open_append(&dir, "checkpoints")?;
        assert_eq!(mode(&dir.join("checkpoints")), 0o600);
        Ok(())
    }

    /// A link planted in the cache is never chmod'ed through: its target keeps its mode.
    /// Also catches the same for a planted link where a file about to be appended goes.
    #[test]
    fn migration_never_changes_a_link_target() -> io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let victim = tmp.path().join("victim");
        fs::write(&victim, "keep")?;
        fs::set_permissions(&victim, fs::Permissions::from_mode(FILE_MODE))?;
        let dir = tmp.path().join("gitterrace");
        fs::create_dir(&dir)?;
        symlink(&victim, dir.join("viewed"))?;
        symlink(&victim, dir.join("checkpoints"))?;
        cache_dir(&dir)?;
        assert_eq!(mode(&victim), FILE_MODE);
        assert!(open_append(&dir, "checkpoints").is_ok());
        assert_eq!(mode(&victim), FILE_MODE);
        assert_eq!(fs::read_to_string(&victim)?, "keep");
        Ok(())
    }
}
