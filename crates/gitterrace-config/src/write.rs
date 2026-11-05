//! Writing a settings file from the settings page: validate first, then replace the file in one
//! rename, keeping one rolling `.bak`. Any refusal or error before the rename leaves the file
//! byte for byte as it was, so a bad save can never break a running server's next reload.

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use crate::locate::{Located, located};

/// Largest settings file the page reads or writes. A real config is a few KiB.
/// Not configurable: a bound on a body a browser page sends to the local server.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// What a file looked like when it was read: its size and modification time.
///
/// A save carries the stamp it started from and is refused when the file moved since, so a
/// browser tab never overwrites an edit made in an editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub len: u64,
    pub mtime_ns: u128,
}

impl Stamp {
    /// The stamp of `path` now. `None` when there is no file.
    pub fn of(path: &Path) -> io::Result<Option<Self>> {
        match fs::symlink_metadata(path) {
            Ok(m) => {
                let mtime_ns = m
                    .modified()?
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos());
                Ok(Some(Self {
                    len: m.len(),
                    mtime_ns,
                }))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The form an HTTP tag carries: `<len>-<mtime_ns>`, or `none` for no file.
    pub fn tag(s: Option<Self>) -> String {
        s.map_or_else(
            || "none".to_owned(),
            |s| format!("{}-{}", s.len, s.mtime_ns),
        )
    }

    /// Reads [`Stamp::tag`] back. `None` for text that is no tag.
    pub fn parse_tag(t: &str) -> Option<Option<Self>> {
        if t == "none" {
            return Some(None);
        }
        let (len, ns) = t.split_once('-')?;
        Some(Some(Self {
            len: len.parse().ok()?,
            mtime_ns: ns.parse().ok()?,
        }))
    }
}

/// Why a save was refused. The file is unchanged in every case.
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    /// The text does not load: the error with its line.
    #[error("{0}")]
    Invalid(Located),
    /// The file changed on disk since the page read it.
    #[error("changed on disk since it was read, reload it")]
    Stale,
    /// The target is a symlink or not a regular file: never written through.
    #[error("{}: not a regular file", .0.display())]
    NotRegular(PathBuf),
    /// The text is larger than [`MAX_FILE_BYTES`].
    #[error("larger than {MAX_FILE_BYTES} bytes")]
    TooLarge,
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// A save that happened: the file's new stamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Saved {
    pub stamp: Stamp,
}

/// Saves `config.toml` in `dir`: the text must load with [`crate::parse`].
pub fn save_config(dir: &Path, text: &str, expect: Option<Stamp>) -> Result<Saved, SaveError> {
    save(&dir.join("config.toml"), text, expect, |t| {
        crate::parse(t).map(drop).map_err(|e| located(t, &e))
    })
}

/// Replaces `target` with `text` when `check` accepts it and the file still has the stamp
/// `expect` (`None`: there must be no file yet).
///
/// Order: check, write `<name>.tmp` (0600, never an existing entry) and sync it, keep the
/// current file as `<name>.bak`, rename the temp over the target, sync the folder.
pub fn save(
    target: &Path,
    text: &str,
    expect: Option<Stamp>,
    check: impl FnOnce(&str) -> Result<(), Located>,
) -> Result<Saved, SaveError> {
    if text.len() > MAX_FILE_BYTES {
        return Err(SaveError::TooLarge);
    }
    check(text).map_err(SaveError::Invalid)?;
    let (dir, name) = split(target)?;
    let current = match fs::symlink_metadata(target) {
        Ok(m) if m.file_type().is_file() => Some(fs::read(target)?),
        Ok(_) => return Err(SaveError::NotRegular(target.to_path_buf())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };
    if Stamp::of(target)? != expect {
        return Err(SaveError::Stale);
    }
    let tmp = temp_file(dir, name, text.as_bytes())?;
    let done = current
        .map_or(Ok(()), |old| backup(dir, name, &old))
        .and_then(|()| fs::rename(&tmp, target));
    if let Err(e) = done {
        // Best effort: the temp name is this process's alone.
        drop(fs::remove_file(&tmp));
        return Err(e.into());
    }
    sync_dir(dir)?;
    let stamp = Stamp::of(target)?.ok_or_else(|| io::Error::other("saved file vanished"))?;
    Ok(Saved { stamp })
}

fn split(target: &Path) -> io::Result<(&Path, &str)> {
    let dir = target
        .parent()
        .ok_or_else(|| io::Error::other("no folder"))?;
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("no file name"))?;
    Ok((dir, name))
}

/// The one rolling backup, replaced by rename so a planted link at the name is replaced, not
/// written through.
fn backup(dir: &Path, name: &str, old: &[u8]) -> io::Result<()> {
    let tmp = temp_file(dir, name, old)?;
    let r = fs::rename(&tmp, dir.join(format!("{name}.bak")));
    if r.is_err() {
        drop(fs::remove_file(&tmp));
    }
    r
}

/// A new private file next to the target holding `bytes`, synced. `create_new` refuses any
/// existing entry, a link included.
fn temp_file(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
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
        o.mode(0o600);
    }
    // An open that fails created nothing, so only a failed write cleans up.
    let mut f = o.open(&tmp)?;
    match f.write_all(bytes).and_then(|()| f.sync_all()) {
        Ok(()) => Ok(tmp),
        Err(e) => {
            drop(fs::remove_file(&tmp));
            Err(e)
        }
    }
}

/// Makes the rename durable. Windows cannot open a folder for sync, and its rename is.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}
