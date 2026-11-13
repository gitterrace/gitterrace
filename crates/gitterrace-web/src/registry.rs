//! Repositories this user has served, so `git terrace open` can find a clone a link names
//! without crawling the disk. `$XDG_STATE_HOME/gitterrace/repos.json`, else
//! `~/.local/state/gitterrace/repos.json` (macOS too). It only ever holds paths the user
//! served and their ids, never anything from a link. The file is 0600 in a 0700 folder,
//! written whole through a temp file and a rename, and a symlink in its place is refused.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gitterrace_git::{Repo, refused};
use serde::{Deserialize, Serialize};

const FILE: &str = "repos.json";
/// Not configurable: a size bound on a state file. Room for thousands of entries, and a
/// longer file was not written by gitterrace.
const MAX_FILE: u64 = 1 << 20;
/// Not configurable: an add token is a few dozen bytes.
const MAX_TOKEN: u64 = 4096;
/// Default of `limits.registry_entries`. Each serve touches one per repository; past this the
/// least recently served go, so a file a user never looks at cannot grow without bound.
static ENTRIES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(500);

/// Sets how many repositories `repos.json` keeps (`limits.registry_entries`). Read once at
/// startup: the file is shared by every server of this user.
pub(crate) fn set_entries(n: usize) {
    ENTRIES.store(n, std::sync::atomic::Ordering::Relaxed);
}

/// Held across every read-modify-write of the file in this process: a startup record and a
/// name claim run on different threads, and the later write would drop the other's entry.
/// Across processes there is no file lock (accepted for 0.1.0): two servers sharing the state
/// folder that write between each other's read and rename lose the earlier write, so two
/// folders claimed in the same instant could both get one name, or a name could go unrecorded
/// and be picked again on the next start. The re-read right before each write narrows the
/// window to that read-to-rename span.
static WRITING: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn writing() -> std::sync::MutexGuard<'static, ()> {
    WRITING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Empty for a repository without commits, which has no id yet.
    #[serde(default)]
    pub id: String,
    pub path: PathBuf,
    /// Unix seconds.
    #[serde(default)]
    pub last_served: u64,
    /// The `/r/NAME` the repository is served under, assigned once and kept, so its links
    /// survive restarts and other repositories joining. Empty until first assigned.
    #[serde(default)]
    pub name: String,
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
/// A missing file is empty; a corrupt one is too, since the next serve rewrites it.
pub fn candidates(dir: &Path) -> io::Result<Vec<Entry>> {
    Ok(gitterrace_git::read_no_follow_bounded(dir, FILE, MAX_FILE)?
        .map(|b| serde_json::from_slice(&b).unwrap_or_default())
        .unwrap_or_default())
}

/// Records `repos` as served now. A repository without commits has no id and is skipped.
pub fn record(dir: &Path, repos: &[&Repo]) -> io::Result<()> {
    let now = now();
    // Ids first: reading one walks the whole history, and the read-modify-write below must be
    // short, since `claim` writes the same file.
    let ids: Vec<(String, PathBuf)> = repos
        .iter()
        .filter_map(|r| Some((r.id()?, r.root())))
        .collect();
    let _held = writing();
    let mut all = candidates(dir)?;
    for (id, path) in ids {
        let name = all
            .iter()
            .find(|e| e.path == path)
            .map(|e| e.name.clone())
            .unwrap_or_default();
        all.retain(|e| e.path != path);
        all.push(Entry {
            id,
            path,
            last_served: now,
            name,
        });
    }
    write_all(dir, all)
}

/// The URL name `root` is served under: the one recorded for it, else a new one recorded now.
///
/// First come keeps a name: a newcomer whose folder name `stem` is already
/// recorded for another folder, or is in `taken` (names served right now), gets
/// `stem-<6 hex of its path>` instead, and an assigned name never changes. Names compare
/// case-insensitively, as macOS and Windows folders do. The file is re-read here, right before
/// the write, so two servers sharing it decide against the same record.
pub(crate) fn claim(
    dir: &Path,
    root: &Path,
    stem: &str,
    taken: &dyn Fn(&str) -> bool,
) -> io::Result<String> {
    let _held = writing();
    let mut all = candidates(dir)?;
    if let Some(e) = all.iter().find(|e| e.path == root && !e.name.is_empty()) {
        return Ok(e.name.clone());
    }
    let used = |n: &str| {
        taken(n)
            || all
                .iter()
                .any(|e| e.path != root && e.name.eq_ignore_ascii_case(n))
    };
    let name = pick(stem, root, &used);
    match all.iter_mut().find(|e| e.path == root) {
        Some(e) => e.name.clone_from(&name),
        None => all.push(Entry {
            id: String::new(),
            path: root.to_path_buf(),
            last_served: now(),
            name: name.clone(),
        }),
    }
    write_all(dir, all)?;
    Ok(name)
}

/// Drops the claim on `root` made by [`claim`] for a repository that was never served: one
/// with no id recorded, so a served repository keeps its name.
pub(crate) fn forget(dir: &Path, root: &Path) -> io::Result<()> {
    let _held = writing();
    let mut all = candidates(dir)?;
    let before = all.len();
    all.retain(|e| e.path != root || !e.id.is_empty());
    if all.len() == before {
        return Ok(());
    }
    write_all(dir, all)
}

/// `stem` when `used` does not hold it, else `stem-<6 hex of root>`, then with a counter on
/// the (unlikely) clash of two hashes. Never depends on arrival order beyond who came first.
pub(crate) fn pick(stem: &str, root: &Path, used: &dyn Fn(&str) -> bool) -> String {
    if !used(stem) {
        return stem.to_owned();
    }
    let hashed = format!("{stem}-{}", short_hash(root));
    let mut name = hashed.clone();
    let mut n = 2;
    while used(&name) {
        name = format!("{hashed}-{n}");
        n += 1;
    }
    name
}

/// Six hex digits of FNV-1a over the path bytes: stable across runs and Rust releases, unlike
/// the std hasher.
fn short_hash(path: &Path) -> String {
    let h = gitterrace_git::fnv1a64(path.as_os_str().as_encoded_bytes().iter().copied());
    format!("{:06x}", h & 0xff_ffff)
}

fn now() -> u64 {
    u64::try_from(gitterrace_index::wall_clock()).unwrap_or(0)
}

/// The `keep` most recently served entries, newest first.
fn newest(mut all: Vec<Entry>, keep: usize) -> Vec<Entry> {
    all.sort_by_key(|e| std::cmp::Reverse(e.last_served));
    all.truncate(keep);
    all
}

/// Newest first, capped. Past `limits.registry_entries` the least recently served go with their
/// names, so a repository unseen for that long may come back under another name if its old one was
/// taken.
fn write_all(dir: &Path, all: Vec<Entry>) -> io::Result<()> {
    let all = newest(all, ENTRIES.load(std::sync::atomic::Ordering::Relaxed));
    let json = serde_json::to_vec_pretty(&all).map_err(io::Error::other)?;
    private_dir(dir)?;
    write_atomic(dir, FILE, &json)
}

/// The file holding the secret `POST /_repos` checks for the server on `port`.
fn token_file(port: u16) -> String {
    format!("add-token-{port}")
}

/// Writes the add secret for the server on `port`: 0600 in the 0700 folder, never through a
/// symlink. Another user cannot read it, so cannot add a repository.
pub(crate) fn write_add_token(dir: &Path, port: u16, token: &str) -> io::Result<()> {
    private_dir(dir)?;
    write_atomic(dir, &token_file(port), token.as_bytes())
}

/// Best effort at shutdown; a stale file only holds a secret no server checks.
pub(crate) fn remove_add_token(dir: &Path, port: u16) {
    drop(fs::remove_file(dir.join(token_file(port))));
}

/// The add secret of the server on `port`, read with the same refusals as the registry: a
/// symlink, a non-file, or (on unix) a file another user owns or can read is refused.
pub fn add_token(dir: &Path, port: u16) -> io::Result<String> {
    let name = token_file(port);
    let bytes = gitterrace_git::read_no_follow_bounded(dir, &name, MAX_TOKEN)?
        .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let path = dir.join(&name);
        let m = fs::symlink_metadata(&path)?;
        let me = rustix::process::getuid().as_raw();
        if m.uid() != me || m.mode() & 0o077 != 0 {
            return Err(io::Error::other(format!(
                "refusing {}: not private to this user",
                path.display()
            )));
        }
    }
    let head = &bytes[..bytes.len().min(256)];
    let token = String::from_utf8_lossy(head).trim().to_owned();
    // The token goes verbatim into a hand-built request header: only base64url passes.
    let base64url = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_');
    if token.is_empty() || !token.bytes().all(base64url) {
        return Err(io::Error::other("token contains unexpected characters"));
    }
    Ok(token)
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

/// 0600, through the shared no-follow temp-and-rename writer. A symlink planted at the name is
/// refused rather than replaced, so the user sees it.
fn write_atomic(dir: &Path, file: &str, bytes: &[u8]) -> io::Result<()> {
    gitterrace_git::write_atomic(dir, file, bytes, 0o600)
}

#[cfg(test)]
mod tests {
    use gitterrace_git::Repo;
    use std::path::Path;

    use crate::registry::{
        Entry, FILE, MAX_FILE, MAX_TOKEN, add_token, candidates, claim, newest, record, token_file,
        write_add_token,
    };

    /// `limits.registry_entries` is how many entries the file keeps: the newest.
    #[test]
    fn registry_entries_keeps_the_newest() {
        let e = |t: u64| Entry {
            id: String::new(),
            path: t.to_string().into(),
            last_served: t,
            name: String::new(),
        };
        let kept: Vec<u64> = newest(vec![e(1), e(3), e(2)], 2)
            .iter()
            .map(|x| x.last_served)
            .collect();
        assert_eq!(kept, [3, 2]);
    }

    /// A one-commit repository; `msg` makes its root commit, and so its id, distinct.
    pub(crate) fn repo(dir: &Path, msg: &str) -> Repo {
        let _fixture = gitterrace_testkit::Fixture::init(dir).commit(msg);
        Repo::discover(dir).expect("repo")
    }

    /// `GITTERRACE_TEST_NOW` freezes the stamp `record` writes, so the equivalence oracle never
    /// sees the wall clock. Runs itself in a child, as a test may not set its own environment.
    #[test]
    fn the_frozen_clock_reaches_the_registry() {
        const NOW: &str = "1710028800";
        if std::env::var("GITTERRACE_TEST_NOW").is_ok_and(|v| v == NOW) {
            let t = tempfile::tempdir().expect("tmp");
            let state = t.path().join("state");
            let a = repo(&t.path().join("a"), "a");
            record(&state, &[&a]).expect("record");
            let got = candidates(&state).expect("read");
            assert_eq!(
                got[0].last_served, 1_710_028_800,
                "registry clock not frozen"
            );
            return;
        }
        let ok = std::process::Command::new(std::env::current_exe().expect("exe"))
            .args([
                "registry::tests::the_frozen_clock_reaches_the_registry",
                "--exact",
            ])
            .env("GITTERRACE_TEST_NOW", NOW)
            .status()
            .expect("child")
            .success();
        assert!(ok, "the child with the clock frozen failed");
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

    /// A name, once claimed, survives every later record of the same folder, from this thread
    /// or another: records rebuild their entry and must carry the name across.
    #[test]
    fn a_claimed_name_survives_records() {
        let t = tempfile::tempdir().expect("tmp");
        let state = t.path().join("state");
        let a = repo(&t.path().join("a"), "a");
        let none = |_: &str| false;
        assert_eq!(claim(&state, &a.root(), "a", &none).expect("claim"), "a");
        let (s2, a2) = (state.clone(), a.root());
        let racer = std::thread::spawn(move || {
            let r = Repo::discover(&a2).expect("repo");
            for _ in 0..20 {
                record(&s2, &[&r]).expect("record");
            }
        });
        for _ in 0..20 {
            assert_eq!(claim(&state, &a.root(), "a", &none).expect("claim"), "a");
        }
        racer.join().expect("join");
        let names: Vec<_> = candidates(&state)
            .expect("read")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["a"]);
    }

    #[test]
    fn add_token_round_trips_privately() {
        let t = tempfile::tempdir().expect("tmp");
        let state = t.path().join("s");
        write_add_token(&state, 4321, "secret").expect("write");
        assert_eq!(add_token(&state, 4321).expect("read"), "secret");
        assert!(add_token(&state, 4322).is_err(), "per port");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let f = state.join("add-token-4321");
            assert_eq!(
                std::fs::metadata(&f).expect("m").permissions().mode() & 0o777,
                0o600
            );
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).expect("chmod");
            assert!(add_token(&state, 4321).is_err(), "readable by others");
            std::fs::remove_file(&f).expect("rm");
            let elsewhere = t.path().join("x");
            std::fs::write(&elsewhere, "planted").expect("w");
            std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o600))
                .expect("chmod");
            std::os::unix::fs::symlink(&elsewhere, &f).expect("link");
            assert!(add_token(&state, 4321).is_err(), "symlink");
            assert!(write_add_token(&state, 4321, "s").is_err(), "not replaced");
        }
    }

    /// The token goes verbatim into a request header, so a line break or a space in the file
    /// must fail the read, not split the header.
    #[test]
    fn add_token_rejects_non_base64url_content() {
        let t = tempfile::tempdir().expect("tmp");
        let state = t.path().join("s");
        write_add_token(&state, 4321, "Ab-_09\n").expect("write");
        assert_eq!(add_token(&state, 4321).expect("read"), "Ab-_09");
        for bad in ["ab\r\nX-Evil: 1", "a b", "a=b", "é", ""] {
            write_add_token(&state, 4321, bad).expect("write");
            assert!(add_token(&state, 4321).is_err(), "{bad:?}");
        }
    }

    /// The class: a state file read whole into memory. Every no-follow read names a bound, and
    /// a file past it (a planted or runaway one) is refused, not read.
    #[test]
    fn a_registry_or_token_file_past_its_bound_is_refused() {
        let t = tempfile::tempdir().expect("tmp");
        let big = usize::try_from(MAX_FILE + 1).expect("fits");
        std::fs::write(t.path().join(FILE), vec![b' '; big]).expect("big");
        assert!(candidates(t.path()).is_err(), "a huge registry is not read");
        let big = usize::try_from(MAX_TOKEN + 1).expect("fits");
        std::fs::write(t.path().join(token_file(1)), vec![b'a'; big]).expect("big token");
        assert!(add_token(t.path(), 1).is_err(), "a huge token is not read");
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
