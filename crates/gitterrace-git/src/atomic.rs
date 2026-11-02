//! The one way gitterrace writes a file. A repository can arrive as a tarball or zip, so any
//! entry it ships (`.git/info`, `.git/info/exclude`, an export folder's leaf) may be a planted
//! symlink, and following one would overwrite a file outside the repository. Every write is a
//! fresh temp file created with `O_NOFOLLOW` in the same folder, then a rename: a crash leaves
//! the old file whole, and a link at the name is refused rather than written through.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Reads `dir/name` without following a link at either. A missing file reads as `None`.
///
/// # Errors
/// When `dir` is not a real folder, `name` is not a regular file, or the read fails.
pub fn read_no_follow(dir: &Path, name: &str) -> io::Result<Option<Vec<u8>>> {
    // A folder not made yet holds no file: the same answer as a missing file.
    if matches!(fs::symlink_metadata(dir), Err(e) if e.kind() == io::ErrorKind::NotFound) {
        return Ok(None);
    }
    let path = regular_entry(dir, name)?;
    match no_follow(OpenOptions::new().read(true)).open(&path) {
        Ok(mut f) => {
            let mut out = Vec::new();
            f.read_to_end(&mut out)?;
            Ok(Some(out))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Replaces `dir/name` with `bytes`: a uniquely named temp file (created new, `O_NOFOLLOW`,
/// `mode` on unix, synced), then a rename in the same folder.
///
/// # Errors
/// When `dir` is not a real folder, an entry at `name` is not a regular file, or the write fails.
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8], mode: u32) -> io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let path = regular_entry(dir, name)?;
    let tmp = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let written = no_follow(&mut o)
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, &path));
    if written.is_err() {
        // Best effort: the temp name is this process's alone.
        drop(fs::remove_file(&tmp));
    }
    written
}

/// Opens `dir/name` for appending and reading, creating it (`O_NOFOLLOW|O_APPEND`, `mode` on
/// unix). A link at the name is refused rather than written through.
///
/// # Errors
/// When `dir` is not a real folder, an entry at `name` is not a regular file, or the open fails.
pub fn open_append_no_follow(dir: &Path, name: &str, mode: u32) -> io::Result<fs::File> {
    let path = regular_entry(dir, name)?;
    let mut o = OpenOptions::new();
    o.create(true).append(true).read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    no_follow(&mut o).open(&path)
}

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
        "refusing to write {}: not a regular file or folder",
        path.display()
    ))
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;

    use super::{read_no_follow, write_atomic};

    #[test]
    fn writes_replace_and_read_back() {
        let t = tempfile::tempdir().expect("tmp");
        write_atomic(t.path(), "f", b"one", 0o644).expect("write");
        write_atomic(t.path(), "f", b"two", 0o644).expect("replace");
        assert_eq!(
            read_no_follow(t.path(), "f").expect("read"),
            Some(b"two".to_vec())
        );
        assert_eq!(read_no_follow(t.path(), "none").expect("read"), None);
    }

    #[test]
    fn a_link_at_the_file_or_the_folder_is_refused_and_its_target_kept() {
        let t = tempfile::tempdir().expect("tmp");
        let victim = t.path().join("victim");
        std::fs::write(&victim, "keep").expect("seed");
        let dir = t.path().join("d");
        std::fs::create_dir(&dir).expect("mkdir");
        symlink(&victim, dir.join("f")).expect("link");
        assert!(write_atomic(&dir, "f", b"x", 0o644).is_err());
        assert!(read_no_follow(&dir, "f").is_err());
        let linked = t.path().join("linked");
        symlink(&dir, &linked).expect("link dir");
        assert!(write_atomic(&linked, "g", b"x", 0o644).is_err());
        assert!(!dir.join("g").exists());
        assert_eq!(std::fs::read_to_string(&victim).expect("read"), "keep");
    }
}
