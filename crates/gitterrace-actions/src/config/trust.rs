//! Where a config may live: ownership, permissions and not inside a served repository.

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::config::{ConfigError, MAX_CONFIG_BYTES};

fn unsafe_path(path: &Path, why: impl Into<String>) -> ConfigError {
    ConfigError::Unsafe { path: path.to_path_buf(), why: why.into() }
}

/// Regular file (not a symlink), owned by us, not group or world writable; the same for
/// its directory; and not inside a served repository.
pub(super) fn check_file(path: &Path, repos: &[PathBuf]) -> Result<(), ConfigError> {
    #[cfg(unix)]
    let uid = rustix::process::getuid().as_raw();
    #[cfg(windows)]
    let uid = 0;
    let meta = std::fs::symlink_metadata(path).map_err(|e| ConfigError::Io(path.into(), e))?;
    if !meta.file_type().is_file() {
        return Err(unsafe_path(path, "not a regular file"));
    }
    if meta.len() > MAX_CONFIG_BYTES {
        return Err(unsafe_path(path, "too large"));
    }
    owned_private(path, &meta, uid)?;
    let real = path.canonicalize().map_err(|e| ConfigError::Io(path.into(), e))?;
    let dir = real.parent().ok_or_else(|| unsafe_path(path, "no parent dir"))?;
    let dmeta = std::fs::metadata(dir).map_err(|e| ConfigError::Io(dir.into(), e))?;
    owned_private(dir, &dmeta, uid)?;
    for root in repos {
        let root = root.canonicalize().unwrap_or_else(|_| root.clone());
        if real.starts_with(&root) {
            return Err(unsafe_path(path, "inside a served repository"));
        }
    }
    Ok(())
}

/// Windows has no mode bits and this does not read ACLs: the file lives under the user's
/// profile (`%APPDATA%`), which is private by default, so the owner check is skipped.
#[cfg(windows)]
#[expect(clippy::unnecessary_wraps, reason = "the signature must match the unix variant")]
const fn owned_private(_: &Path, _: &std::fs::Metadata, _: u32) -> Result<(), ConfigError> {
    Ok(())
}

#[cfg(unix)]
fn owned_private(path: &Path, meta: &std::fs::Metadata, uid: u32) -> Result<(), ConfigError> {
    if meta.uid() != uid {
        return Err(unsafe_path(path, "not owned by the current user"));
    }
    if meta.permissions().mode() & 0o022 != 0 {
        return Err(unsafe_path(path, "group or world writable"));
    }
    Ok(())
}
