/// Pinned author and committer date for every commit in this module.
pub(super) const T0: &str = "2024-01-01T00:00:00+00:00";

use gitterrace_testkit::git_at;
use std::collections::BTreeMap;
use std::fs;
use std::sync::Arc;

use super::BlameCache;
use crate::{ChangeKind, LineKind, Repo};

pub(super) fn write(dir: &std::path::Path, path: &str, body: &str) {
    let full = dir.join(path);
    fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
    fs::write(full, body).expect("write");
}

pub(super) fn commit(dir: &std::path::Path, msg: &str) {
    git_at(dir, T0, &["add", "-A"]);
    git_at(dir, T0, &["commit", "-q", "-m", msg]);
}

pub(super) fn numbered(n: usize) -> String {
    (1..=n).fold(String::new(), |acc, i| acc + &format!("line {i}\n"))
}

/// Guard class: a path log that stops once its page is full must list exactly the slice
/// `git log -- path` lists, on every page. Two branches with interleaved commit dates (so a
/// commit reached later in the walk can be dated before one already listed), a merge that
/// takes one side (git hides the other) and a merge that combines both (git shows both).
/// Red when the early stop cuts a page short or reorders it.
#[test]
fn every_page_of_a_path_log_is_the_slice_git_lists() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    let at = |m: u32| format!("2024-01-01T{:02}:{:02}:00+00:00", m / 60, m % 60);
    let step = |d: &std::path::Path, m: u32, files: &[(&str, String)], msg: &str| {
        for (p, body) in files {
            write(d, p, body);
        }
        git_at(d, &at(m), &["add", "-A"]);
        git_at(d, &at(m), &["commit", "-q", "-m", msg]);
    };
    git_at(d, &at(0), &["init", "-q", "-b", "main"]);
    step(
        d,
        0,
        &[("p/a.txt", "0\n".into()), ("other.txt", "0\n".into())],
        "root",
    );
    git_at(d, &at(1), &["checkout", "-q", "-b", "side"]);
    git_at(d, &at(1), &["checkout", "-q", "main"]);
    // Dates interleave: main 10, side 15, main 20, side 25, main 30, side 35.
    for (i, m) in [10, 20, 30].into_iter().enumerate() {
        step(
            d,
            m,
            &[("p/a.txt", format!("main {i}\n"))],
            &format!("main {i}"),
        );
        git_at(d, &at(m + 5), &["checkout", "-q", "side"]);
        step(
            d,
            m + 5,
            &[
                ("p/b.txt", format!("side {i}\n")),
                ("other.txt", format!("s{i}\n")),
            ],
            &format!("side {i}"),
        );
        git_at(d, &at(m + 5), &["checkout", "-q", "main"]);
    }
    // Combines both sides: git shows the side's commits for p and other.txt.
    git_at(d, &at(40), &["merge", "-q", "--no-edit", "side"]);
    git_at(d, &at(41), &["checkout", "-q", "-b", "hidden", "HEAD~1"]);
    step(d, 42, &[("p/a.txt", "hidden\n".into())], "hidden");
    git_at(d, &at(43), &["checkout", "-q", "main"]);
    // Takes main's tree: git hides the hidden branch for p/a.txt.
    git_at(
        d,
        &at(44),
        &["merge", "-q", "-s", "ours", "--no-edit", "hidden"],
    );
    step(d, 50, &[("p/a.txt", "after\n".into())], "after");
    let repo = Repo::discover(d).expect("repo");
    for path in ["p/a.txt", "p", "p/b.txt", "other.txt"] {
        let want: Vec<String> =
            gitterrace_testkit::git(d, &["log", "--format=%H", "HEAD", "--", path])
                .lines()
                .map(str::to_owned)
                .collect();
        assert!(!want.is_empty(), "{path}");
        for limit in 1..=want.len() + 1 {
            for skip in 0..=want.len() {
                let got: Vec<String> = repo
                    .log("HEAD", Some(path), skip, limit)
                    .expect("log")
                    .into_iter()
                    .map(|c| c.id)
                    .collect();
                let slice: Vec<String> = want.iter().skip(skip).take(limit).cloned().collect();
                assert_eq!(got, slice, "{path} skip {skip} limit {limit}");
            }
        }
    }
}

/// A committed-only handle answers every committed form the pages link to and refuses
/// every rev that reaches local state or enumerates objects, through each public reader.
#[test]
fn committed_only_refuses_local_state() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    write(d, "a.txt", "one\n");
    commit(d, "first");
    write(d, "a.txt", "two\n");
    commit(d, "second");
    git_at(d, T0, &["tag", "v1"]);
    git_at(d, T0, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git_at(d, T0, &["update-ref", "refs/notes/x", "HEAD"]);
    write(d, "a.txt", "secret wip\n");
    git_at(d, T0, &["stash", "-q"]);
    let open = Repo::discover(d).expect("repo");
    let head = open.resolve("HEAD").expect("head");
    let stash = open.resolve("stash").expect("stash resolves locally");
    let repo = Repo::discover(d).expect("repo").committed_only();
    for ok in [
        "HEAD",
        "main",
        "refs/heads/main",
        "v1",
        "refs/tags/v1",
        "origin/main",
        "HEAD~1",
        "main^",
        "HEAD^0",
        head.as_str(),
    ] {
        assert!(repo.resolve(ok).is_ok(), "{ok} refused");
    }
    assert_eq!(
        repo.resolve("HEAD~1").expect("restricted"),
        open.resolve("HEAD~1").expect("open")
    );
    // A full id that no public ref reaches is refused too, see `tests_visible.rs`.
    for bad in [
        "stash",
        "refs/stash",
        &stash[..12],
        &head[..7],
        "refs/notes/x",
        "main@{1}",
        ":/second",
        "HEAD:a.txt",
        "main..HEAD",
        "HEAD~9",
        "ORIG_HEAD",
        "FETCH_HEAD",
        "",
    ] {
        assert!(repo.resolve(bad).is_err(), "{bad:?} resolved");
        assert!(repo.blob(bad, "a.txt").is_err(), "{bad:?} blob");
        assert!(repo.commit(bad).is_err(), "{bad:?} commit");
        assert!(repo.log(bad, None, 0, 5).is_err(), "{bad:?} log");
    }
}

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let d = tmp.path();
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    git_at(d, T0, &["config", "user.name", "Old Name"]);
    git_at(d, T0, &["config", "user.email", "old@example.com"]);
    write(
        d,
        ".mailmap",
        "New Name <new@example.com> <old@example.com>\n",
    );
    write(d, "a.txt", &numbered(40));
    write(d, "src/b.txt", "one\ntwo\n");
    commit(d, "feat: first\n\nbody text\nmore body");
    let mut edited = numbered(40)
        .replace("line 5\n", "five\n")
        .replace("line 30\n", "thirty\n");
    edited.push_str("tail\n");
    write(d, "a.txt", &edited);
    write(d, "bin.dat", "\0\u{1}\u{2}binary");
    commit(d, "fix: edit a, add binary");
    fs::rename(d.join("src/b.txt"), d.join("src/c.txt")).expect("rename");
    commit(d, "refactor: rename b to c");
    fs::remove_file(d.join("a.txt")).expect("rm");
    write(d, "src/c.txt", "one\ntwo\nthree\n");
    commit(d, "chore: drop a, grow c");
    tmp
}

/// Parses `git log --numstat -z -M` into id to path to `(add, del, old_path)`.
#[expect(
    clippy::type_complexity,
    reason = "test reference shape, a named type adds nothing"
)]
fn reference(
    dir: &std::path::Path,
) -> BTreeMap<String, BTreeMap<String, (u32, u32, Option<String>)>> {
    let raw = git_at(
        dir,
        T0,
        &[
            "log",
            "--numstat",
            "-z",
            "-M",
            "--format=%x01%H%x00",
            "--topo-order",
        ],
    );
    let mut out = BTreeMap::new();
    for chunk in raw.split('\u{1}').filter(|c| !c.is_empty()) {
        let mut parts = chunk.split('\0');
        let id = parts.next().expect("id").to_string();
        let mut files = BTreeMap::new();
        while let Some(head) = parts.next() {
            let head = head.trim_start_matches('\n');
            if head.is_empty() {
                continue;
            }
            let mut f = head.split('\t');
            let (a, d, p) = (
                f.next().expect("a"),
                f.next().expect("d"),
                f.next().expect("p"),
            );
            let (path, old) = if p.is_empty() {
                let old = parts.next().expect("old").to_string();
                (parts.next().expect("new").to_string(), Some(old))
            } else {
                (p.to_string(), None)
            };
            let num = |s: &str| s.parse().unwrap_or(0);
            files.insert(path, (num(a), num(d), old));
        }
        out.insert(id, files);
    }
    out
}

#[test]
fn numstat_matches_git() {
    let tmp = fixture();
    let repo = Repo::discover(tmp.path()).expect("open");
    let want = reference(tmp.path());
    assert_eq!(want.len(), 4);
    for id in repo.rev_list("HEAD").expect("rev_list") {
        let got: BTreeMap<_, _> = repo
            .numstat_many(std::slice::from_ref(&id))
            .expect("numstat")
            .remove(0)
            .expect("readable")
            .into_iter()
            .map(|f| (f.path, (f.additions, f.deletions, f.old_path)))
            .collect();
        assert_eq!(got, want[&id], "commit {id}");
    }
}

#[test]
fn commit_detail_has_hunks_kinds_and_body() {
    let tmp = fixture();
    let repo = Repo::discover(tmp.path()).expect("open");
    let log = repo.log("HEAD", None, 0, 10).expect("log");
    assert_eq!(log.len(), 4);
    let second = &log[2];
    assert_eq!(second.subject, "fix: edit a, add binary");
    let detail = repo.commit(&second.id).expect("commit");
    let a = detail
        .files
        .iter()
        .find(|f| f.path == "a.txt")
        .expect("a.txt");
    assert_eq!(
        (a.kind, a.additions, a.deletions),
        (ChangeKind::Modified, 3, 2)
    );
    assert_eq!(a.hunks.len(), 3);
    let first = &a.hunks[0];
    assert_eq!(first.header, "@@ -2,7 +2,7 @@");
    let removed = first
        .lines
        .iter()
        .find(|l| l.kind == LineKind::Removed)
        .expect("removed");
    assert_eq!(
        (removed.old_no, removed.new_no, removed.text.as_str()),
        (Some(5), None, "line 5")
    );
    let bin = detail
        .files
        .iter()
        .find(|f| f.path == "bin.dat")
        .expect("bin");
    assert!(bin.binary && bin.hunks.is_empty() && bin.kind == ChangeKind::Added);

    let root = repo.commit(&log[3].id).expect("root");
    assert_eq!(root.body, "body text\nmore body");
    assert!(root.files.iter().all(|f| f.kind == ChangeKind::Added));

    let rename = repo.commit(&log[1].id).expect("rename");
    assert_eq!(rename.files.len(), 1);
    assert_eq!(rename.files[0].kind, ChangeKind::Renamed);
    assert_eq!(rename.files[0].old_path.as_deref(), Some("src/b.txt"));
}

#[test]
fn log_orders_filters_and_pages() {
    let tmp = fixture();
    let repo = Repo::discover(tmp.path()).expect("open");
    let all = repo.log("main", None, 0, 100).expect("log");
    assert_eq!(all[0].subject, "chore: drop a, grow c");
    let paged = repo.log("main", None, 1, 2).expect("log");
    assert_eq!(paged.len(), 2);
    assert_eq!(paged[0].id, all[1].id);
    let on_a = repo.log("HEAD", Some("a.txt"), 0, 100).expect("log");
    let subjects: Vec<_> = on_a.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(
        subjects,
        [
            "chore: drop a, grow c",
            "fix: edit a, add binary",
            "feat: first"
        ]
    );
    let oldest_first = repo.rev_list("HEAD").expect("rev_list");
    assert_eq!(oldest_first.last(), Some(&all[0].id));
    assert!(matches!(
        repo.resolve("nope"),
        Err(crate::Error::RevNotFound(_))
    ));
    // History searches and reflog reads never resolve, whichever route passes them.
    for rev in [":/first", "HEAD^{/first}", "main@{1}", "@{-1}"] {
        assert!(
            matches!(repo.resolve(rev), Err(crate::Error::RevNotFound(_))),
            "{rev}"
        );
        assert!(repo.log(rev, None, 0, 1).is_err(), "{rev}");
    }
    assert!(repo.resolve("HEAD~1").is_ok());
}

#[test]
fn mailmap_is_applied() {
    let tmp = fixture();
    let repo = Repo::discover(tmp.path()).expect("open");
    let log = repo.log("HEAD", None, 0, 1).expect("log");
    assert_eq!(log[0].author.name, "New Name");
    assert_eq!(log[0].author.email, "new@example.com");
}

#[test]
fn tree_blob_blame_refs() {
    let tmp = fixture();
    let d = tmp.path();
    git_at(d, T0, &["tag", "v1"]);
    let repo = Repo::discover(d).expect("open");
    let first = repo.log("HEAD", None, 0, 10).expect("log")[3].id.clone();
    let entries = repo.tree(&first, "").expect("tree");
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["src", ".mailmap", "a.txt"]);
    assert_eq!(repo.tree("HEAD", "src").expect("src").len(), 1);
    assert!(repo.tree("HEAD", "missing").is_err());
    let blob = repo.blob(&first, "src/b.txt").expect("blob");
    assert_eq!(blob.text.as_deref(), Some("one\ntwo\n"));
    assert!(repo.blob("HEAD", "src/c.txt").is_ok());
    let second = repo.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
    assert!(repo.blob(&second, "bin.dat").expect("bin").text.is_none());

    let blame = repo.blame(&second, "a.txt").expect("blame");
    assert_eq!(blame.len(), 41);
    assert_eq!(blame[4].text, "five");
    assert_ne!(blame[4].commit, blame[0].commit);
    assert_eq!(blame[0].author, "New Name");

    assert_eq!(repo.head_branch().as_deref(), Some("main"));
    let refs = repo.refs().expect("refs");
    let kinds: Vec<_> = refs.iter().map(|r| (r.name.as_str(), r.kind)).collect();
    assert!(kinds.contains(&("main", crate::RefKind::Branch)));
    assert!(kinds.contains(&("v1", crate::RefKind::Tag)));
    assert_ne!(repo.name(), "");
}

#[test]
fn a_lower_text_cap_hides_text_and_a_small_blame_cache_keeps_nothing() {
    let tmp = fixture();
    let d = tmp.path();
    let open = Repo::discover(d).expect("open");
    let at = open.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
    assert!(open.blob(&at, "a.txt").expect("blob").text.is_some());
    let tiny = Repo::discover(d).expect("open").with_limits(10, 1);
    assert_eq!(tiny.text_limit(), 10);
    let blob = tiny.blob(&at, "a.txt").expect("blob");
    assert!(blob.size > 10 && blob.text.is_none(), "over the cap");
    assert!(tiny.blame(&at, "a.txt").expect("blame").is_empty());
    let mut cache = BlameCache::with_limit(1);
    let key = (
        gix::ObjectId::empty_tree(gix::hash::Kind::Sha1),
        "p".to_owned(),
    );
    cache.put(
        key.clone(),
        Arc::new(open.blame(&at, "a.txt").expect("blame")),
    );
    assert!(
        cache.get(&key).is_none(),
        "a result over the budget is not kept"
    );
    let mut roomy = BlameCache::with_limit(1 << 20);
    roomy.put(
        key.clone(),
        Arc::new(open.blame(&at, "a.txt").expect("blame")),
    );
    assert!(roomy.get(&key).is_some());
}

#[test]
fn blame_cache_follows_the_commit() {
    let tmp = fixture();
    let d = tmp.path();
    let repo = Repo::discover(d).expect("open");
    let second = repo.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
    assert!(!repo.blame_is_cached(&second, "a.txt"));
    let before = repo.blame(&second, "a.txt").expect("blame");
    assert!(
        repo.blame_is_cached(&second, "a.txt"),
        "a repeat blame is a hit"
    );
    assert_eq!(
        repo.blame(&second, "a.txt").expect("cached").len(),
        before.len()
    );
    git_at(d, T0, &["checkout", "-q", "-b", "other", &second]);
    std::fs::write(d.join("a.txt"), "only\n").expect("write");
    git_at(d, T0, &["commit", "-qam", "shrink"]);
    assert!(
        !repo.blame_is_cached("other", "a.txt"),
        "a new commit misses"
    );
    let after = repo.blame("other", "a.txt").expect("blame");
    assert_eq!((before.len(), after.len()), (41, 1));
    assert_eq!(repo.blame(&second, "a.txt").expect("old").len(), 41);
}

#[test]
fn blobs_over_the_text_cap_are_never_read_as_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let d = tmp.path();
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    git_at(d, T0, &["config", "user.name", "A"]);
    git_at(d, T0, &["config", "user.email", "a@x"]);
    let size = crate::MAX_TEXT_BYTES as usize + 2;
    write(d, "big.txt", &"a\n".repeat(size / 2));
    commit(d, "add big");
    write(d, "big.txt", &"b\n".repeat(size / 2));
    commit(d, "rewrite big");
    let repo = Repo::discover(d).expect("open");
    let blob = repo.blob("HEAD", "big.txt").expect("blob");
    assert_eq!((blob.size, blob.text), (size as u64, None));
    assert!(repo.blame("HEAD", "big.txt").expect("blame").is_empty());
    let file = &repo.commit("HEAD").expect("commit").files[0];
    assert!(file.binary && file.hunks.is_empty());
    assert_eq!((file.additions, file.deletions), (0, 0));
}

#[test]
fn a_mode_only_change_carries_both_modes() {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    git_at(d, T0, &["init", "-q", "-b", "main"]);
    write(d, "run.sh", "echo\n");
    commit(d, "init");
    git_at(d, T0, &["update-index", "--chmod=+x", "run.sh"]);
    git_at(d, T0, &["commit", "-q", "-m", "chore: make it executable"]);
    let repo = Repo::discover(d).expect("open");
    let head = repo.resolve("HEAD").expect("head");
    let c = repo.commit(&head).expect("commit");
    assert_eq!(c.files.len(), 1);
    assert_eq!(c.files[0].modes, Some((0o100_644, 0o100_755)));
    assert!(c.files[0].hunks.is_empty());
}

/// Guard class: any memo of ref data must miss when a ref is added, moved or deleted. Red when
/// the ref list served after such a change is the one built before it.
#[test]
fn the_ref_list_follows_every_new_moved_and_deleted_ref() {
    let tmp = fixture();
    let d = tmp.path();
    let repo = Repo::discover(d).expect("open");
    let names = |r: &Repo| -> Vec<(String, String)> {
        let mut v: Vec<_> = r
            .refs()
            .expect("refs")
            .into_iter()
            .map(|x| (x.name, x.target))
            .collect();
        v.sort();
        v
    };
    let before = names(&repo);
    assert_eq!(before, names(&repo), "a repeat answers the same list");
    git_at(d, T0, &["tag", "v9"]);
    assert!(
        names(&repo).iter().any(|(n, _)| n == "v9"),
        "a new tag shows"
    );
    git_at(d, T0, &["branch", "side", "HEAD~1"]);
    let side = names(&repo)
        .into_iter()
        .find(|(n, _)| n == "side")
        .expect("side")
        .1;
    git_at(d, T0, &["branch", "-f", "side", "HEAD"]);
    let moved = names(&repo)
        .into_iter()
        .find(|(n, _)| n == "side")
        .expect("side")
        .1;
    assert_ne!(side, moved, "a moved branch shows its new commit");
    git_at(d, T0, &["tag", "-d", "v9"]);
    assert!(
        !names(&repo).iter().any(|(n, _)| n == "v9"),
        "a deleted tag goes"
    );
    git_at(d, T0, &["pack-refs", "--all"]);
    assert_eq!(
        names(&repo).len(),
        before.len() + 1,
        "packing changes nothing shown"
    );
}

/// Guard class: a combined read answers what the separate reads answer, on every side of
/// both caps. Red when the text or bytes of one read differ from `blob` and `blob_bytes`.
#[test]
fn one_read_of_text_and_bytes_matches_the_two_reads() {
    let tmp = fixture();
    let d = tmp.path();
    let repo = Repo::discover(d).expect("open");
    let at = repo.log("HEAD", None, 0, 10).expect("log")[2].id.clone();
    for (path, cap) in [
        ("a.txt", 1 << 20),
        ("a.txt", 4),
        ("bin.dat", 1 << 20),
        ("bin.dat", 0),
    ] {
        let (b, bytes) = repo.blob_with_bytes(&at, path, cap).expect("one");
        let two = repo.blob(&at, path).expect("blob");
        assert_eq!((b.size, b.text), (two.size, two.text), "{path} {cap}");
        let want = if b.size <= cap {
            repo.blob_bytes(&at, path).expect("bytes")
        } else {
            Vec::new()
        };
        assert_eq!(bytes, want, "{path} {cap}");
    }
}
