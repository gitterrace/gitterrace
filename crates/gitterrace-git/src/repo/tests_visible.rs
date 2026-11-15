//! A committed-only handle answers a full id only for a commit reachable from a branch, tag,
//! remote-tracking ref or HEAD, judged on the commit the whole revision lands on.

use gitterrace_testkit::git_at;

use crate::Repo;
use crate::repo::tests::{T0, commit, write};

/// Full ids of one repository's public and hidden commits.
struct Ids {
    old: String,
    tip: String,
    remote_only: String,
    tag_only: String,
    tag_object: String,
    detached: String,
    stash: String,
    stash_index: String,
    dangling: String,
    blob: String,
}

/// `main` with three commits, a commit only `origin/feat` reaches, one only an annotated tag
/// reaches, a stash on `main`, a dangling commit, and HEAD detached on a commit of its own.
fn repo() -> (tempfile::TempDir, Ids) {
    let tmp = tempfile::tempdir().expect("tmp");
    let d = tmp.path();
    let g = |args: &[&str]| git_at(d, T0, args);
    g(&["init", "-q", "-b", "main"]);
    write(d, "a.txt", "one\n");
    commit(d, "first");
    let old = g(&["rev-parse", "HEAD"]);
    g(&["checkout", "-q", "-b", "feat"]);
    write(d, "r.txt", "remote\n");
    commit(d, "remote side");
    let remote_only = g(&["rev-parse", "HEAD"]);
    g(&["update-ref", "refs/remotes/origin/feat", "HEAD"]);
    g(&["checkout", "-q", "-b", "rel", "main"]);
    write(d, "t.txt", "tagged\n");
    commit(d, "tag side");
    let tag_only = g(&["rev-parse", "HEAD"]);
    g(&["tag", "-a", "v1", "-m", "v1"]);
    let tag_object = g(&["rev-parse", "v1"]);
    g(&["checkout", "-q", "main"]);
    g(&["branch", "-q", "-D", "feat", "rel"]);
    write(d, "a.txt", "two\n");
    commit(d, "second");
    write(d, "a.txt", "three\n");
    commit(d, "third");
    let tip = g(&["rev-parse", "HEAD"]);
    write(d, "a.txt", "secret wip\n");
    g(&["stash", "-q"]);
    let stash = g(&["rev-parse", "stash"]);
    let stash_index = g(&["rev-parse", "stash^2"]);
    let tree = g(&["rev-parse", "stash:"]);
    let dangling = g(&["commit-tree", &tree, "-p", &tip, "-m", "dangling"]);
    let blob = g(&["rev-parse", "stash:a.txt"]);
    g(&["checkout", "-q", "--detach"]);
    write(d, "h.txt", "head\n");
    commit(d, "detached");
    let detached = g(&["rev-parse", "HEAD"]);
    let ids = Ids {
        old,
        tip,
        remote_only,
        tag_only,
        tag_object,
        detached,
        stash,
        stash_index,
        dangling,
        blob,
    };
    (tmp, ids)
}

#[test]
fn a_full_id_answers_only_for_a_commit_a_public_root_reaches() {
    let (tmp, ids) = repo();
    let repo = Repo::discover(tmp.path()).expect("repo").committed_only();
    let allowed = [
        (ids.old.clone(), &ids.old),
        (ids.tip.clone(), &ids.tip),
        (ids.remote_only.clone(), &ids.remote_only),
        (ids.tag_only.clone(), &ids.tag_only),
        (ids.tag_object.clone(), &ids.tag_only),
        (ids.detached.clone(), &ids.detached),
        (format!("{}~1", ids.detached), &ids.tip),
        (format!("{}^", ids.stash), &ids.tip),
        (format!("{}~3", ids.dangling), &ids.old),
        ("v1".to_owned(), &ids.tag_only),
        ("origin/feat".to_owned(), &ids.remote_only),
        ("HEAD".to_owned(), &ids.detached),
        ("main~2".to_owned(), &ids.old),
    ];
    for (rev, want) in &allowed {
        assert_eq!(
            repo.resolve(rev).ok().as_ref(),
            Some(*want),
            "{rev} refused"
        );
        assert!(repo.has(rev, None) || rev.len() != 40, "{rev} has");
    }
    let zeros = "0".repeat(40);
    let not_hex = "g".repeat(40);
    let denied = [
        ids.stash.clone(),
        ids.stash_index.clone(),
        ids.dangling.clone(),
        format!("{}^0", ids.stash),
        format!("{}^2", ids.stash),
        format!("{}~0", ids.dangling),
        ids.blob.clone(),
        zeros,
        not_hex,
    ];
    for rev in &denied {
        assert!(repo.resolve(rev).is_err(), "{rev} resolved");
        assert!(repo.blob(rev, "a.txt").is_err(), "{rev} blob");
        assert!(repo.tree(rev, "").is_err(), "{rev} tree");
        assert!(repo.commit(rev).is_err(), "{rev} commit");
        assert!(repo.log(rev, None, 0, 5).is_err(), "{rev} log");
        assert!(repo.range(&ids.tip, rev, 5).is_err(), "{rev} range head");
        assert!(repo.range(rev, &ids.tip, 5).is_err(), "{rev} range base");
        assert!(!repo.has(rev, None), "{rev} has");
    }
}

#[test]
fn a_local_handle_still_reads_a_stash_by_its_id() {
    let (tmp, ids) = repo();
    let repo = Repo::discover(tmp.path()).expect("repo");
    for rev in [&ids.stash, &ids.stash_index, &ids.dangling] {
        assert_eq!(repo.resolve(rev).ok().as_ref(), Some(rev), "{rev}");
    }
    assert_eq!(
        repo.blob(&ids.stash, "a.txt")
            .expect("blob")
            .text
            .as_deref(),
        Some("secret wip\n")
    );
}

/// The reachable set follows the refs: a branch made at a hidden commit shows it, and
/// deleting that branch hides it again, though the set only grew in between.
#[test]
fn the_visible_set_follows_a_ref_that_moves_or_goes() {
    let (tmp, ids) = repo();
    let d = tmp.path();
    let repo = Repo::discover(d).expect("repo").committed_only();
    assert!(repo.resolve(&ids.dangling).is_err());
    git_at(d, T0, &["branch", "-q", "keep", &ids.dangling]);
    assert_eq!(
        repo.resolve(&ids.dangling).ok().as_ref(),
        Some(&ids.dangling)
    );
    git_at(d, T0, &["branch", "-q", "-D", "keep"]);
    assert!(
        repo.resolve(&ids.dangling).is_err(),
        "deleted ref still shows"
    );
    git_at(
        d,
        T0,
        &["update-ref", "refs/remotes/origin/feat", &ids.stash],
    );
    assert_eq!(repo.resolve(&ids.stash).ok().as_ref(), Some(&ids.stash));
    assert!(
        repo.resolve(&ids.remote_only).is_err(),
        "moved ref keeps its old history"
    );
}

/// One walk serves every later id while the refs stay put, and a ref or HEAD resolves
/// without building the set at all.
#[test]
fn the_visible_set_is_walked_once_per_ref_state() {
    let (tmp, ids) = repo();
    let repo = Repo::discover(tmp.path()).expect("repo").committed_only();
    for rev in ["HEAD", "main", "v1~1", &ids.detached] {
        assert!(repo.resolve(rev).is_ok(), "{rev}");
    }
    assert_eq!(
        repo.visible.lock().expect("lock").len(),
        0,
        "a root needs no walk"
    );
    assert!(repo.resolve(&ids.old).is_ok());
    let built = repo.visible.lock().expect("lock").len();
    assert!(built > 0, "an old id builds the set");
    for rev in [&ids.stash, &ids.tip, &ids.dangling, &ids.old] {
        let _resolved = repo.resolve(rev).is_ok();
        assert_eq!(
            repo.visible.lock().expect("lock").len(),
            built,
            "{rev} rewalked"
        );
    }
}

/// The set costs an 8-byte prefix per commit and nothing for a hash table: at most
/// `BYTES_PER_COMMIT` heap bytes for each commit it holds, so a 1.3M-commit history stays near
/// 10 MB. Full 20-byte ids fail it.
#[test]
fn the_visible_set_costs_one_id_per_commit() {
    const BYTES_PER_COMMIT: usize = 8;
    let (tmp, ids) = repo();
    let repo = Repo::discover(tmp.path()).expect("repo").committed_only();
    assert!(repo.resolve(&ids.old).is_ok());
    let v = repo.visible.lock().expect("lock");
    assert!(v.len() > 0, "an old id builds the set");
    assert!(
        v.heap_bytes() <= v.len() * BYTES_PER_COMMIT,
        "{} bytes for {} commits",
        v.heap_bytes(),
        v.len()
    );
}

/// Clearing drops every commit and says it held some; the next full id walks again and the
/// answers stay the same, a hidden commit refused as before.
#[test]
fn a_cleared_visible_set_rebuilds_on_the_next_full_id() {
    let (tmp, ids) = repo();
    let repo = Repo::discover(tmp.path()).expect("repo").committed_only();
    assert!(!repo.clear_visible(), "an unbuilt set holds nothing");
    assert!(repo.resolve(&ids.old).is_ok());
    assert!(repo.clear_visible(), "a built set reports what it held");
    assert_eq!(repo.visible.lock().expect("lock").len(), 0);
    assert!(repo.resolve(&ids.dangling).is_err(), "cleared set widened");
    assert!(repo.resolve(&ids.old).is_ok());
    assert!(repo.visible.lock().expect("lock").len() > 0, "not rebuilt");
}

/// An unreachable id whose 8-byte prefix a reachable commit shares is refused by the exact
/// walk, and a reachable id on a shared prefix is still served.
#[test]
fn a_shared_prefix_refuses_the_unreachable_id_and_serves_the_reachable_one() {
    let (tmp, ids) = repo();
    let repo = Repo::discover(tmp.path()).expect("repo").committed_only();
    assert!(repo.resolve(&ids.old).is_ok(), "an old id builds the set");
    let dangling = gix::ObjectId::from_hex(ids.dangling.as_bytes()).expect("hex");
    repo.visible
        .lock()
        .expect("lock")
        .inject_collision(dangling);
    assert!(
        repo.resolve(&ids.dangling).is_err(),
        "a shared prefix served a hidden commit"
    );
    assert!(repo.resolve(&ids.stash).is_err(), "a stash leaked");
    assert!(
        repo.resolve(&ids.old).is_ok(),
        "a shared prefix refused a reachable commit"
    );
}
