//! Repositories this user has served, so `git terrace open` can find a clone a link names
//! without crawling the disk. `$XDG_STATE_HOME/gitterrace/repos.json`, else
//! `~/.local/state/gitterrace/repos.json` (macOS too). It only ever holds paths the user
//! served and their ids, never anything from a link. The file is 0600 in a 0700 folder,
//! written whole through a temp file and a rename, and a symlink in its place is refused.

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gitterrace_git::Repo;
use serde::{Deserialize, Serialize};

const FILE: &str = "repos.json";
/// Entries kept. Each serve touches one per repository; past this the least recently served
/// go, so a file a user never looks at cannot grow without bound.
const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub path: PathBuf,
    /// Unix seconds.
    pub last_served: u64,
}

/// The registry folder: absolute `$XDG_STATE_HOME/gitterrace`, else
/// `$HOME/.local/state/gitterrace`; `%LOCALAPPDATA%\gitterrace` on Windows.
pub fn state_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(a) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            return Some(a.join("gitterrace"));
        }
    }
    let xdg = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from);
    if let Some(x) = xdg.filter(|p| p.is_absolute()) {
        return Some(x.join("gitterrace"));
    }
    gitterrace_config::home_var().map(|h| PathBuf::from(h).join(".local/state/gitterrace"))
}

/// Every recorded entry, unchecked.
///
/// A path may now hold another project, and reading an id
/// walks the whole history (seconds on a large clone). A caller narrows by the recorded id,
/// then reads the candidate's own id before trusting it.
pub fn candidates(dir: &Path) -> io::Result<Vec<Entry>> {
    read_raw(dir)
}

/// Records `repos` as served now. A repository without commits has no id and is skipped.
pub fn record(dir: &Path, repos: &[&Repo]) -> io::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let mut all = read_raw(dir)?;
    for r in repos {
        let Some(id) = r.id() else { continue };
        let path = r.root();
        all.retain(|e| e.path != path);
        all.push(Entry {
            id,
            path,
            last_served: now,
        });
    }
    all.sort_by_key(|e| std::cmp::Reverse(e.last_served));
    all.truncate(MAX_ENTRIES);
    let json = serde_json::to_vec_pretty(&all).map_err(io::Error::other)?;
    private_dir(dir)?;
    write_atomic(dir, &json)
}

/// Every entry as written, unchecked. A missing file is empty; a corrupt one is too, since
/// the next serve rewrites it.
fn read_raw(dir: &Path) -> io::Result<Vec<Entry>> {
    let path = dir.join(FILE);
    match no_follow(OpenOptions::new().read(true)).open(&path) {
        Ok(mut f) => {
            if !f.metadata()?.is_file() {
                return Err(refused(&path));
            }
            let mut s = String::new();
            f.read_to_string(&mut s)?;
            Ok(serde_json::from_str(&s).unwrap_or_default())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) if is_loop(&e) => Err(refused(&path)),
        Err(e) => Err(e),
    }
}

fn private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let m = fs::symlink_metadata(dir)?;
    if !m.is_dir() {
        return Err(refused(dir));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// A fresh temp file (0600, `O_NOFOLLOW`, created new), then a rename over the old file. A
/// symlink planted at the name is refused rather than replaced, so the user sees it.
fn write_atomic(dir: &Path, bytes: &[u8]) -> io::Result<()> {
    let path = dir.join(FILE);
    if fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file()) {
        return Err(refused(&path));
    }
    let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
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

#[cfg(unix)]
fn no_follow(o: &mut OpenOptions) -> &mut OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    o.custom_flags(libc::O_NOFOLLOW)
}

#[cfg(not(unix))]
fn no_follow(o: &mut OpenOptions) -> &mut OpenOptions {
    o
}

/// `O_NOFOLLOW` on a symlink fails with `ELOOP`.
#[cfg(unix)]
fn is_loop(e: &io::Error) -> bool {
    e.raw_os_error() == Some(libc::ELOOP)
}

#[cfg(not(unix))]
fn is_loop(_: &io::Error) -> bool {
    false
}

fn refused(path: &Path) -> io::Error {
    io::Error::other(format!(
        "refusing {}: not a regular file or folder",
        path.display()
    ))
}

#[cfg(test)]
mod tests {
    use gitterrace_git::Repo;
    use std::path::Path;

    use crate::registry::{FILE, candidates, record};

    /// A one-commit repository; `msg` makes its root commit, and so its id, distinct.
    pub(crate) fn repo(dir: &Path, msg: &str) -> Repo {
        let _fixture = gitterrace_testkit::Fixture::init(dir).commit(msg);
        Repo::discover(dir).expect("repo")
    }

    #[test]
    fn records_atomically_private_and_drops_stale() {
        let t = tempfile::tempdir().expect("tmp");
        let state = t.path().join("state/gitterrace");
        let a = repo(&t.path().join("a"), "a");
        let b = repo(&t.path().join("b"), "b");
        record(&state, &[&a, &b]).expect("record");
        let got = candidates(&state).expect("read");
        assert_eq!(got.len(), 2);
        assert!(
            got.iter()
                .any(|e| e.path == a.root() && Some(&e.id) == a.id().as_ref())
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&state.join(FILE)), 0o600);
            assert_eq!(mode(&state), 0o700);
        }
        let leftovers: Vec<_> = std::fs::read_dir(&state)
            .expect("ls")
            .filter_map(Result::ok)
            .filter(|e| e.file_name() != FILE)
            .collect();
        assert!(leftovers.is_empty(), "no temp file left behind");

        // b is gone; a's folder now holds another project with another id.
        std::fs::remove_dir_all(b.root()).expect("rm");
        std::fs::remove_dir_all(a.root()).expect("rm");
        repo(&a.root(), "another project");
        assert_eq!(candidates(&state).expect("raw").len(), 2, "kept on disk");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_file() {
        let t = tempfile::tempdir().expect("tmp");
        let state = t.path().join("s");
        std::fs::create_dir_all(&state).expect("dir");
        let target = t.path().join("elsewhere.json");
        std::fs::write(&target, "[]").expect("w");
        std::os::unix::fs::symlink(&target, state.join(FILE)).expect("link");
        let a = repo(&t.path().join("a"), "a");
        assert!(record(&state, &[&a]).is_err(), "write refused");
        assert!(candidates(&state).is_err(), "read refused");
        assert_eq!(
            std::fs::read_to_string(&target).expect("r"),
            "[]",
            "target untouched"
        );
    }
}
