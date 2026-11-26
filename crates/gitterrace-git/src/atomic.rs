//! The one way gitterrace writes a file. A repository can arrive as a tarball or zip, so any
//! entry it ships (`.git/info`, `.git/info/exclude`, an export folder's leaf) may be a planted
//! symlink, and following one would overwrite a file outside the repository. Every write is a
//! fresh temp file created with `O_NOFOLLOW` in the same folder, then a rename: a crash leaves
//! the old file whole, and a link at the name is refused rather than written through.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Reads `dir/name` without following a link at either, up to `max` bytes.
///
/// A longer file is refused with `InvalidData`, so a planted huge file cannot be read into
/// memory. The limit holds even when the file grows after the open. A missing file reads as
/// `None`. There is no unbounded variant: every reader names what its file can hold.
///
/// # Errors
/// When `dir` is not a real folder, `name` is not a regular file, the read fails, or the file
/// is longer than `max`.
pub fn read_no_follow_bounded(dir: &Path, name: &str, max: u64) -> io::Result<Option<Vec<u8>>> {
    let Some(f) = open_read_no_follow(dir, name)? else {
        return Ok(None);
    };
    if f.metadata()?.len() > max {
        return Err(too_long(dir, name));
    }
    let mut out = Vec::new();
    f.take(max.saturating_add(1)).read_to_end(&mut out)?;
    if out.len() as u64 > max {
        return Err(too_long(dir, name));
    }
    Ok(Some(out))
}

/// Opens `dir/name` for reading, never following a link or blocking on a FIFO.
///
/// The open is non-blocking and no-follow, and the opened handle must be a regular
/// file. Checking the handle, not the path, closes the race with an entry swapped after a
/// path check. A missing file opens as `None`.
///
/// # Errors
/// When `dir` is not a real folder, the entry is a link or not a regular file, or the open
/// fails.
pub fn open_read_no_follow(dir: &Path, name: &str) -> io::Result<Option<fs::File>> {
    // A folder not made yet holds no file: the same answer as a missing file.
    if matches!(fs::symlink_metadata(dir), Err(e) if e.kind() == io::ErrorKind::NotFound) {
        return Ok(None);
    }
    let path = regular_entry(dir, name)?;
    let mut o = OpenOptions::new();
    o.read(true);
    let f = match read_flags(&mut o).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !f.metadata()?.is_file() {
        return Err(refused(&path));
    }
    Ok(Some(f))
}

fn too_long(dir: &Path, name: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{} is longer than expected", dir.join(name).display()),
    )
}

/// No-follow and non-blocking on unix (a FIFO opens at once and is then refused as not a
/// regular file). On Windows the reparse point itself is opened, so a link is seen as one.
#[cfg(unix)]
fn read_flags(o: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
}

#[cfg(not(unix))]
fn read_flags(o: &mut OpenOptions) -> &mut OpenOptions {
    no_follow(o)
}

/// Replaces `dir/name` with `bytes`: a uniquely named temp file (created new, `O_NOFOLLOW`,
/// `mode` on unix, synced), then a rename in the same folder.
///
/// # Errors
/// When `dir` is not a real folder, an entry at `name` is not a regular file, or the write fails.
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
    write_replacing(dir, name, bytes, mode, true)
}

/// [`write_atomic`] without the sync, for a folder discarded unless the whole job finishes.
///
/// An export's staging folder is one: a per-file flush there buys nothing and costs a full
/// device flush per file on macOS.
///
/// # Errors
/// As [`write_atomic`].
pub fn write_atomic_unsynced(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
    write_replacing(dir, name, bytes, mode, false)
}

/// Files synced on this thread. Per thread, so a test bounds its own job's flushes while
/// other tests write in parallel.
pub fn synced_files() -> u64 {
    SYNCED.with(std::cell::Cell::get)
}

thread_local! {
    static SYNCED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

fn write_replacing(dir: &Path, name: &str, bytes: &[u8], mode: u32, sync: bool) -> io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let path = regular_entry(dir, name)?;
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let written = create_new_no_follow(&tmp, mode)
        .and_then(|mut f| {
            f.write_all(bytes)?;
            if sync {
                SYNCED.with(|n| n.set(n.get() + 1));
                f.sync_all()?;
            }
            Ok(())
        })
        .and_then(|()| fs::rename(&tmp, &path));
    if written.is_err() {
        // Best effort: the temp name is this process's alone.
        drop(fs::remove_file(&tmp));
    }
    written
}

/// Opens `dir/name` for appending and reading, creating it, and refuses a link or a shared inode.
///
/// The open is `O_NOFOLLOW|O_APPEND` with `mode` on unix. A link at the name is refused rather
/// than written through, and so is a file with other hard links: an append would reach the same
/// inode through the other name, outside the private folder. The check is on the opened handle,
/// so a swap after the path check is caught.
///
/// # Errors
/// When `dir` is not a real folder, an entry at `name` is not a regular file, it has other hard
/// links, or the open fails.
pub fn open_append_no_follow(dir: &Path, name: &str, mode: u32) -> io::Result<fs::File> {
    let path = regular_entry(dir, name)?;
    let mut o = OpenOptions::new();
    o.create(true).append(true).read(true);
    set_mode(&mut o, mode);
    let f = no_follow(&mut o).open(&path)?;
    #[cfg(unix)]
    refuse_shared(&f, &path)?;
    Ok(f)
}

/// Refuses a handle whose inode is shared under another name. Nothing was written yet. Unix
/// only, where the link count is read from the handle.
#[cfg(unix)]
fn refuse_shared(f: &fs::File, path: &Path) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = f.metadata()?;
    if !m.is_file() || m.nlink() > 1 {
        return Err(io::Error::other(format!(
            "refusing to append to {}: it is not a regular file or has other hard links",
            path.display()
        )));
    }
    Ok(())
}

/// Creates `path` new for writing, `O_NOFOLLOW` and `mode` on unix, so a link or an existing
/// file at the path is an error, never written through.
///
/// # Errors
/// When the path exists or the open fails.
pub fn create_new_no_follow(path: &Path, mode: u32) -> io::Result<fs::File> {
    let mut o = OpenOptions::new();
    o.read(true).write(true).create_new(true);
    set_mode(&mut o, mode);
    no_follow(&mut o).open(path)
}

/// Removes group and other access from an existing file or folder.
///
/// The handle is opened `O_NOFOLLOW`, so a planted link is never chmod'ed (`fchmod` on the
/// opened inode, not `chmod` on the path). A link, a special file or a file with other hard
/// links is left alone: its inode may belong to somebody else. Does nothing off unix.
///
/// # Errors
/// When the open or the mode change fails. A missing path is not an error.
#[cfg(unix)]
pub fn make_private_no_follow(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let mut o = OpenOptions::new();
    o.read(true);
    let f = match read_flags(&mut o).open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        // O_NOFOLLOW on a link: not ours to change.
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Ok(()),
        Err(e) => return Err(e),
    };
    let m = f.metadata()?;
    let shared = m.is_file() && m.nlink() > 1;
    if (!m.is_file() && !m.is_dir()) || shared || m.mode().trailing_zeros() >= 6 {
        return Ok(());
    }
    f.set_permissions(fs::Permissions::from_mode(m.mode() & 0o700))
}

/// Other platforms have no mode bits to tighten.
///
/// # Errors
/// Never.
#[cfg(not(unix))]
pub const fn make_private_no_follow(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The creation mode where the platform has one.
#[cfg(unix)]
fn set_mode(o: &mut OpenOptions, mode: u32) {
    use std::os::unix::fs::OpenOptionsExt;
    o.mode(mode);
}

/// Other platforms have no creation mode.
#[cfg(not(unix))]
const fn set_mode(_o: &mut OpenOptions, _mode: u32) {}

/// `dir/name`, after checking `dir` is a real folder and `name`, when present, a regular file.
fn regular_entry(dir: &Path, name: &str) -> io::Result<PathBuf> {
    if !fs::symlink_metadata(dir)?.is_dir() {
        return Err(refused(dir));
    }
    let path = dir.join(name);
    match fs::symlink_metadata(&path) {
        Ok(m) if !m.is_file() => Err(refused(&path)),
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(path),
    }
}

/// Opens the final component itself, never a link's target: `O_NOFOLLOW` on unix, and on
/// Windows `FILE_FLAG_OPEN_REPARSE_POINT`, so a link or junction opens as itself and a write
/// through it fails. Every no-follow open in this crate sets its flags here.
#[cfg(unix)]
pub(crate) fn no_follow(o: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    o.custom_flags(libc::O_NOFOLLOW)
}

#[cfg(windows)]
pub(crate) fn no_follow(o: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::windows::fs::OpenOptionsExt;
    // FILE_FLAG_OPEN_REPARSE_POINT
    o.custom_flags(0x0020_0000)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn no_follow(o: &mut OpenOptions) -> &mut OpenOptions {
    o
}

/// The error for a path that is not a regular file or folder, which no write follows.
pub fn refused(path: &Path) -> io::Error {
    io::Error::other(format!("refusing to write {}: not a regular file or folder", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{read_no_follow_bounded, write_atomic};

    #[test]
    fn writes_replace_and_read_back() {
        let t = tempfile::tempdir().expect("tmp");
        write_atomic(t.path(), "f", b"one", 0o644).expect("write");
        write_atomic(t.path(), "f", b"two", 0o644).expect("replace");
        assert_eq!(
            read_no_follow_bounded(t.path(), "f", 1024).expect("read"),
            Some(b"two".to_vec())
        );
        assert_eq!(read_no_follow_bounded(t.path(), "none", 1024).expect("read"), None);
    }

    #[cfg(unix)]
    mod unix {
        use std::os::unix::fs::symlink;

        use crate::atomic::{open_read_no_follow, read_no_follow_bounded, write_atomic};

        #[test]
        fn a_link_at_the_file_or_the_folder_is_refused_and_its_target_kept() {
            let t = tempfile::tempdir().expect("tmp");
            let victim = t.path().join("victim");
            std::fs::write(&victim, "keep").expect("seed");
            let dir = t.path().join("d");
            std::fs::create_dir(&dir).expect("mkdir");
            symlink(&victim, dir.join("f")).expect("link");
            assert!(write_atomic(&dir, "f", b"x", 0o644).is_err());
            assert!(read_no_follow_bounded(&dir, "f", 1024).is_err());
            let linked = t.path().join("linked");
            symlink(&dir, &linked).expect("link dir");
            assert!(write_atomic(&linked, "g", b"x", 0o644).is_err());
            assert!(!dir.join("g").exists());
            assert_eq!(std::fs::read_to_string(&victim).expect("read"), "keep");
        }

        #[test]
        fn a_fifo_is_refused_at_once_and_a_long_file_past_its_bound() {
            let t = tempfile::tempdir().expect("tmp");
            // The crate forbids unsafe, so the FIFO comes from the system tool.
            let made = std::process::Command::new("mkfifo")
                .arg(t.path().join("p"))
                .status()
                .expect("mkfifo");
            assert!(made.success(), "mkfifo");
            assert!(open_read_no_follow(t.path(), "p").is_err());
            write_atomic(t.path(), "f", b"12345", 0o644).expect("write");
            assert!(read_no_follow_bounded(t.path(), "f", 4).is_err());
            assert_eq!(
                read_no_follow_bounded(t.path(), "f", 5).expect("read"),
                Some(b"12345".to_vec())
            );
        }
    }

    #[cfg(all(test, windows))]
    mod windows {
        use std::path::Path;

        use crate::atomic::{open_append_no_follow, write_atomic};

        /// Plants a directory junction at `at` pointing to `to`: junctions need no privilege.
        fn junction(at: &Path, to: &Path) -> std::io::Result<()> {
            let made = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(at)
                .arg(to)
                .status()?;
            assert!(made.success(), "mklink /J");
            Ok(())
        }

        #[test]
        fn open_append_refuses_reparse_point() {
            let t = tempfile::tempdir().expect("tmp");
            let target = t.path().join("target");
            std::fs::create_dir(&target).expect("mkdir");
            junction(&t.path().join("f"), &target).expect("junction");
            assert!(open_append_no_follow(t.path(), "f", 0o644).is_err());
        }

        #[test]
        fn write_atomic_refuses_reparse_point() {
            let t = tempfile::tempdir().expect("tmp");
            let target = t.path().join("target");
            std::fs::create_dir(&target).expect("mkdir");
            junction(&t.path().join("f"), &target).expect("junction");
            assert!(write_atomic(t.path(), "f", b"x", 0o644).is_err());
            assert!(std::fs::read_dir(&target).expect("list").next().is_none());
        }
    }
}
