//! Opening and reading one worktree file without leaving the working tree: no link is
//! followed on the way down and the read stops at the size limit.

use std::path::Path;

/// Reads at most `max` bytes from `f`, `None` when it holds more: the size checked on the
/// path is advisory, a file still being written can outgrow it before the read.
pub(super) fn read_bounded(f: &std::fs::File, max: u64) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read as _;
    let mut buf = Vec::new();
    f.take(max.saturating_add(1)).read_to_end(&mut buf)?;
    Ok((buf.len() as u64 <= max).then_some(buf))
}

/// Opens `rel` under `work` one component at a time, each relative to the folder handle
/// before it and none followed if it is a link. The canonicalized parent check in
/// [`super::read_worktree`] is a separate step, so a folder swapped for a link in between would
/// otherwise lead the open outside the working tree.
#[cfg(unix)]
pub(super) fn open_beneath(work: &Path, rel: &str) -> std::io::Result<std::fs::File> {
    use rustix::fs::{CWD, Mode, OFlags, openat};

    let mut parts: Vec<&str> = rel.split('/').filter(|p| !p.is_empty()).collect();
    let Some(last) = parts.pop() else {
        return Err(crate::refused(work));
    };
    if parts.iter().chain([&last]).any(|p| *p == "." || *p == "..") {
        return Err(crate::refused(work));
    }
    let base = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let dir_flags = base | OFlags::DIRECTORY;
    let mut dir = openat(CWD, work, dir_flags, Mode::empty())?;
    for part in parts {
        dir = openat(&dir, part, dir_flags, Mode::empty())?;
    }
    let f = std::fs::File::from(openat(&dir, last, base | OFlags::NONBLOCK, Mode::empty())?);
    if !f.metadata()?.is_file() {
        return Err(crate::refused(&work.join(rel)));
    }
    Ok(f)
}

/// Windows has no `openat`: the canonicalized parent check and the no-follow open of the
/// final component stand, under the same-user threat model.
#[cfg(not(unix))]
pub(super) fn open_beneath(work: &Path, rel: &str) -> std::io::Result<std::fs::File> {
    let path = work.join(rel);
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    let f = crate::atomic::no_follow(&mut opts).open(&path)?;
    if !f.metadata()?.is_file() {
        return Err(crate::refused(&path));
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_grown_past_the_limit_reads_as_big() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path().join("grow.txt");
        std::fs::write(&p, "x".repeat(10)).expect("write");
        let f = std::fs::File::open(&p).expect("open");
        // The size check passed at 10 bytes; the file now holds 11 when it is read.
        std::fs::write(&p, "x".repeat(11)).expect("grow");
        assert_eq!(super::read_bounded(&f, 10).expect("read"), None);
        let f = std::fs::File::open(&p).expect("open");
        assert_eq!(
            super::read_bounded(&f, 11).expect("read").map(|b| b.len()),
            Some(11)
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_parent_folder_swapped_for_a_link_is_not_followed() {
        let outside = tempfile::tempdir().expect("tempdir");
        std::fs::write(outside.path().join("secret.txt"), "secret\n").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        let work = dir.path().canonicalize().expect("canon");
        std::fs::create_dir(work.join("d")).expect("mkdir");
        std::fs::write(work.join("d/secret.txt"), "mine\n").expect("write");
        let f = super::open_beneath(&work, "d/secret.txt").expect("open");
        assert_eq!(
            super::read_bounded(&f, 64).expect("read"),
            Some(b"mine\n".to_vec())
        );
        // The swap a racing writer would make after the parent check.
        std::fs::remove_dir_all(work.join("d")).expect("rm");
        std::os::unix::fs::symlink(outside.path(), work.join("d")).expect("link");
        let err = super::open_beneath(&work, "d/secret.txt").expect_err("a link is refused");
        assert!(
            matches!(err.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)),
            "{err:?}"
        );
        assert!(super::open_beneath(&work, "../x").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn a_symlink_as_the_final_component_is_refused_not_followed() {
        let outside = tempfile::tempdir().expect("tempdir");
        let secret = outside.path().join("secret.txt");
        std::fs::write(&secret, "secret\n").expect("write");
        let dir = tempfile::tempdir().expect("tempdir");
        let work = dir.path().canonicalize().expect("canon");
        std::os::unix::fs::symlink(&secret, work.join("link.png")).expect("link");
        let err = super::open_beneath(&work, "link.png").expect_err("a link is refused");
        assert_eq!(err.raw_os_error(), Some(libc::ELOOP));
    }

    #[test]
    #[cfg(windows)]
    fn open_beneath_refuses_reparse_point() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("target");
        std::fs::create_dir(&target).expect("mkdir");
        let at = dir.path().join("j");
        let made = std::process::Command::new("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&at)
            .arg(&target)
            .status()
            .expect("mklink");
        assert!(made.success(), "mklink /J");
        assert!(super::open_beneath(dir.path(), "j").is_err());
    }
}
