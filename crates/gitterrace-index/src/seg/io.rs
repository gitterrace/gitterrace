//! The one positional read shim and the durable-write helpers. Windows `seek_read` moves the
//! file cursor, so every reader holds its own `File`.

use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Fills `buf` from `offset`, failing with `UnexpectedEof` on a short file.
pub(crate) fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
    }
    #[cfg(windows)]
    {
        let mut done = 0usize;
        while done < buf.len() {
            let at = offset
                .checked_add(done as u64)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "offset overflow"))?;
            match std::os::windows::fs::FileExt::seek_read(file, &mut buf[done..], at) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => done += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Opens the cache file `dir/name` for reading: a missing file, a link or a special file is
/// an error, never followed or blocked on (see `cache::open_read`). Every container and
/// manifest-named file opens here.
pub(crate) fn open_named(dir: &Path, name: &str) -> io::Result<File> {
    crate::cache::open_read(dir, name)?.ok_or_else(|| io::ErrorKind::NotFound.into())
}

/// Makes a rename in `dir` durable. Unix only: Windows has no directory handle to sync, and
/// NTFS journals the rename, so each caller gates the call.
#[cfg(unix)]
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Creates `dir/name` afresh for reading and writing without following a link at that name:
/// the cache folder can come from the repository, so a planted link must fail, never be
/// written through. An existing file is unlinked and a new inode created exclusively, so a
/// planted hard link to another file loses its name instead of truncating that file. Every
/// segment, order, manifest, spool and spill temp file opens here.
pub(crate) fn create_no_follow(dir: &Path, name: &str) -> io::Result<File> {
    let path = crate::cache::cache_file(dir, name)?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    gitterrace_git::create_new_no_follow(&path, crate::cache::FILE_MODE)
}

/// Writes `bytes` to `<name>.tmp`, fsyncs and renames it to `name`, then syncs `dir`.
pub(crate) fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = format!("{name}.tmp");
    {
        let mut f = create_no_follow(dir, &tmp)?;
        io::Write::write_all(&mut f, bytes)?;
        f.sync_all()?;
    }
    fs::rename(dir.join(&tmp), dir.join(name))?;
    #[cfg(unix)]
    sync_dir(dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_at_offsets_and_fails_short() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        write_atomic(dir.path(), "f", b"0123456789")?;
        let f = File::open(dir.path().join("f"))?;
        let mut b = [0; 3];
        read_exact_at(&f, &mut b, 7)?;
        assert_eq!(&b, b"789");
        read_exact_at(&f, &mut b, 2)?;
        assert_eq!(&b, b"234");
        assert!(read_exact_at(&f, &mut b, 8).is_err());
        assert!(read_exact_at(&f, &mut b, u64::MAX).is_err());
        assert!(!dir.path().join("f.tmp").exists());
        Ok(())
    }

    #[test]
    fn temp_write_leaves_a_hard_linked_file_untouched() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let sentinel = dir.path().join("sentinel");
        fs::write(&sentinel, b"keep me")?;
        fs::hard_link(&sentinel, dir.path().join("MANIFEST.tmp"))?;
        fs::hard_link(&sentinel, dir.path().join("f.tmp"))?;
        write_atomic(dir.path(), "f", b"new")?;
        let mut f = create_no_follow(dir.path(), "MANIFEST.tmp")?;
        io::Write::write_all(&mut f, b"overwritten")?;
        assert_eq!(fs::read(&sentinel)?, b"keep me");
        assert_eq!(fs::read(dir.path().join("f"))?, b"new");
        Ok(())
    }

    #[test]
    #[cfg(windows)]
    fn create_no_follow_refuses_reparse_point() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("target");
        fs::create_dir(&target)?;
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(dir.path().join("seg"))
            .arg(&target)
            .status()?;
        assert!(made.success(), "mklink /J");
        assert!(create_no_follow(dir.path(), "seg").is_err());
        Ok(())
    }
}
