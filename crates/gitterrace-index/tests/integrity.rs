//! A corrupt or unsafe cache through the surface pages use: `Index::disk_source` and
//! `Checked::run`. Corruption must surface as an error the caller rebuilds on, never as a
//! commit reported missing or a changed value. A FIFO, device or link in the cache must never
//! hang an open or be followed: those cases run in a child process under a short timeout, so
//! a regression fails here instead of hanging the suite.
#![expect(clippy::expect_used, reason = "test setup")]

use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::{Duration, Instant};

use gitterrace_git::Repo;
use gitterrace_index::feed::{IdentId, InsightsSource, Posting};
use gitterrace_index::seg::format::section;
use gitterrace_index::seg::read::Store;
use gitterrace_index::{CommitSource, Index};
use gitterrace_testkit::Fixture;

/// Set in the child: the cache folder it opens.
const CHILD_DIR: &str = "TEST_INTEGRITY_CHILD_DIR";
#[cfg(unix)]
const CHILD_TIMEOUT: Duration = Duration::from_secs(5);

struct Built {
    _tmp: tempfile::TempDir,
    repo: Repo,
    index: Index,
    cache: PathBuf,
    head: String,
}

fn built() -> Built {
    let tmp = tempfile::tempdir().expect("tmp");
    let f = Fixture::standard(&tmp.path().join("r"));
    let repo = Repo::discover(f.path()).expect("repo");
    let cache = tmp.path().join("cache");
    let index = Index::open_dir(cache.clone()).expect("index");
    index.update(&repo, "HEAD", &|_| {}).expect("update");
    let head = repo.resolve("HEAD").expect("head");
    Built {
        _tmp: tmp,
        repo,
        index,
        cache,
        head,
    }
}

/// The segment file and the offset of the first block of `sec` in it.
fn first_block(cache: &Path, sec: u16) -> (PathBuf, u64) {
    let store = Store::open(cache).expect("store").expect("a set");
    let seg = &store.segments[0];
    (cache.join(seg.name()), seg.blocks(sec)[0].offset)
}

/// Flips one byte in place, the same inode, so an already open handle sees it too.
fn flip(path: &Path, at: u64) {
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open");
    let mut b = [0u8];
    f.seek(SeekFrom::Start(at)).expect("seek");
    std::io::Read::read_exact(&mut f, &mut b).expect("read");
    f.seek(SeekFrom::Start(at)).expect("seek");
    f.write_all(&[b[0] ^ 0xff]).expect("write");
}

fn find_head(b: &Built) -> Result<Option<u32>, String> {
    let set = b
        .index
        .disk_source(&b.repo, &b.head)
        .map_err(|e| e.to_string())?
        .ok_or("no set")?;
    set.run(|s| s.find_commit(&b.head).map(|r| r.map(|r| r.0)))
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[test]
fn a_corrupt_oid_boundary_is_an_error_never_a_missing_head() {
    let b = built();
    assert!(matches!(find_head(&b), Ok(Some(_))), "valid lookup");
    let (seg, at) = first_block(&b.cache, section::OID_INDEX);
    // The first byte of the first oid: the routing boundary of block 0.
    flip(&seg, at);
    let got = find_head(&b);
    assert!(got.is_err(), "a corrupt boundary gave {got:?}");
    // The caller's answer to an unreadable set: rebuild, then the lookup is whole again.
    b.index.rebuild().expect("rebuild");
    b.index.update(&b.repo, "HEAD", &|_| {}).expect("update");
    assert!(matches!(find_head(&b), Ok(Some(_))), "after rebuild");
}

#[test]
fn a_rank_changed_after_open_is_an_error_never_served() {
    let b = built();
    let set = b
        .index
        .disk_source(&b.repo, &b.head)
        .expect("open")
        .expect("set");
    let (seg, at) = first_block(&b.cache, section::RANK);
    flip(&seg, at);
    let got = set.run(|s| {
        s.postings(Posting::Author(IdentId(0)))
            .map(|h| h.rank.0)
            .collect::<Vec<_>>()
    });
    assert!(got.is_err(), "a changed rank was served: {got:?}");
}

/// A child run: opens the cache and the set. The outcome may be an error; only returning
/// matters. Skipped outside a child.
#[test]
fn child_opens_the_cache() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    let dir = PathBuf::from(dir);
    drop(Index::open_dir(dir.clone()));
    drop(Store::open(&dir));
}

#[cfg(unix)]
/// Runs `child_opens_the_cache` on `dir` in a child process, killed after `CHILD_TIMEOUT`.
fn open_in_child(dir: &Path, case: &str) {
    let mut child = Command::new(std::env::current_exe().expect("exe"))
        .args(["--exact", "child_opens_the_cache"])
        .env(CHILD_DIR, dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("wait") {
            assert!(status.success(), "{case}: the child failed");
            return;
        }
        if start.elapsed() > CHILD_TIMEOUT {
            child.kill().expect("kill");
            drop(child.wait());
            panic!("{case}: opening the cache hung");
        }
        gitterrace_testkit::wait::pause_blocking(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn mkfifo(path: &Path) {
    let ok = Command::new("mkfifo").arg(path).status().expect("mkfifo");
    assert!(ok.success(), "mkfifo");
}

/// Every cache entry the reader opens, replaced by a FIFO or a link to a file outside: the
/// open never hangs or follows it, and the cache then rebuilds and answers whole.
#[cfg(unix)]
#[test]
fn a_fifo_or_link_in_the_cache_never_hangs_and_rebuilds() {
    let probe = built();
    let store = Store::open(&probe.cache).expect("store").expect("set");
    let segment = store.segments[0].name().to_owned();
    drop(store);
    for name in ["MANIFEST", "VERSION", segment.as_str()] {
        for kind in ["fifo", "link"] {
            let case = format!("{kind} at {name}");
            let b = built();
            let at = b.cache.join(name);
            fs::remove_file(&at).expect("rm");
            let outside = b.cache.parent().expect("parent").join("outside");
            fs::write(&outside, "keep").expect("outside");
            if kind == "fifo" {
                mkfifo(&at);
            } else {
                std::os::unix::fs::symlink(&outside, &at).expect("link");
            }
            open_in_child(&b.cache, &case);
            assert_eq!(
                fs::read_to_string(&outside).expect("read"),
                "keep",
                "{case}"
            );
            let index = Index::open_dir(b.cache.clone()).expect("reopen");
            let set = index.disk_source(&b.repo, &b.head);
            assert!(
                !matches!(set, Ok(Some(_))) || find_head(&b).is_ok(),
                "{case}: partly trusted"
            );
            index.rebuild().expect("rebuild");
            index.update(&b.repo, "HEAD", &|_| {}).expect("update");
            let rank = index
                .disk_source(&b.repo, &b.head)
                .expect("open")
                .expect("set")
                .run(|s| s.find_commit(&b.head))
                .expect("run")
                .expect("find");
            assert!(rank.is_some(), "{case}: rebuilt set misses HEAD");
            assert!(
                !fs::symlink_metadata(&at).is_ok_and(|m| !m.is_file()),
                "{case}"
            );
        }
    }
}

/// A `VERSION` far longer than any stamp is read only up to its bound, and the cache resets.
#[test]
fn a_huge_version_file_resets_the_cache() {
    let b = built();
    fs::write(b.cache.join("VERSION"), vec![b'v'; 1 << 20]).expect("write");
    let index = Index::open_dir(b.cache.clone()).expect("open");
    let v = fs::read_to_string(b.cache.join("VERSION")).expect("read");
    assert_eq!(v.trim(), format!("v{}", gitterrace_index::FORMAT_VERSION));
    index.update(&b.repo, "HEAD", &|_| {}).expect("update");
    assert!(matches!(find_head(&b), Ok(Some(_))));
    assert!(CommitSource::rev_list(&b.repo, "HEAD").is_ok());
}
