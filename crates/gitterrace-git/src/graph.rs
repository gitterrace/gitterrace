//! The commit-graph a full-history walk reads parents from, so it decodes no commit object.

/// The repository's own commit-graph, or `None` when it has none or the configuration turns it
/// off.
///
/// gix looks only in the repository's own `objects/info`. A graph held by an alternate object
/// store is never read: an alternates entry may name any path on the machine, and its graph
/// would feed parents and roots from outside the served tree. A shared clone still walks, it
/// decodes each commit from the borrowed objects instead.
pub(crate) fn commit_graph(repo: &gix::Repository) -> Option<gix::commitgraph::Graph> {
    repo.commit_graph_if_enabled().ok().flatten()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gitterrace_testkit::git_at;

    use crate::{Repo, Roots};

    const T: &str = "2024-01-01T00:00:00+00:00";

    fn commit(d: &Path, date: &str, msg: &str) -> String {
        git_at(d, date, &["commit", "-q", "--allow-empty", "-m", msg]);
        git_at(d, date, &["rev-parse", "HEAD"])
    }

    fn id(d: &Path) -> Option<String> {
        Repo::discover(d).expect("open").id()
    }

    /// A merged-in second root: the id is the older root, the same with and without a graph.
    #[test]
    fn id_is_the_older_merged_root_with_and_without_a_graph() {
        let tmp = tempfile::tempdir().expect("tmp");
        let d = tmp.path();
        git_at(d, T, &["init", "-q", "-b", "main"]);
        git_at(d, T, &["config", "user.name", "T"]);
        git_at(d, T, &["config", "user.email", "t@example.com"]);
        commit(d, T, "new root");
        git_at(d, T, &["checkout", "-q", "--orphan", "side"]);
        let old = commit(d, "2020-01-01T00:00:00+00:00", "old root");
        git_at(d, T, &["checkout", "-q", "main"]);
        let merge = [
            "merge",
            "-q",
            "--allow-unrelated-histories",
            "-m",
            "m",
            "side",
        ];
        git_at(d, T, &merge);
        commit(d, T, "after");
        let want = Some(old[..12].to_owned());
        assert_eq!(id(d), want, "without a graph");
        git_at(d, T, &["commit-graph", "write", "--reachable"]);
        assert!(d.join(".git/objects/info/commit-graph").is_file());
        assert_eq!(id(d), want, "with a graph");
    }

    /// A `main` of root, three middle commits and tip in `a`, a graph written there, and `b`
    /// cloned from it with `--shared`, so `b` borrows every object through an absolute
    /// alternates entry. Returns the root id and the middle ids.
    fn shared(tmp: &Path) -> (String, Vec<String>) {
        let a = tmp.join("a");
        std::fs::create_dir(&a).expect("dir");
        git_at(&a, T, &["init", "-q", "-b", "main"]);
        git_at(&a, T, &["config", "user.name", "T"]);
        git_at(&a, T, &["config", "user.email", "t@example.com"]);
        let root = commit(&a, T, "root");
        let middle: Vec<String> = (0..3).map(|i| commit(&a, T, &format!("c{i}"))).collect();
        commit(&a, T, "tip");
        git_at(&a, T, &["commit-graph", "write", "--reachable"]);
        let (from, to) = (
            a.to_string_lossy(),
            tmp.join("b").to_string_lossy().into_owned(),
        );
        git_at(tmp, T, &["clone", "-q", "--shared", &from, &to]);
        (root, middle)
    }

    /// A shared clone reads its id and roots through the borrowed objects.
    #[test]
    fn shared_clone_walks_history_via_alternates() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (root, _) = shared(tmp.path());
        let b = tmp.path().join("b");
        assert_eq!(id(&b), Some(root[..12].to_owned()));
        let mut roots = Roots::default();
        roots
            .update(&Repo::discover(&b).expect("open"))
            .expect("roots");
        assert_eq!(roots.ids().iter().collect::<Vec<_>>(), [&root]);
    }

    /// Guard class: nothing a foreign object store's commit-graph says reaches a walk of the
    /// served repository. The middle commit objects are deleted from `a`, so only `a`'s graph
    /// still links the tip to the root. Red when any walk (the id, the roots, the public
    /// reachable set) reads that graph: the root would come out found and public.
    #[test]
    fn an_alternate_commit_graph_outside_the_repo_is_never_read() {
        let tmp = tempfile::tempdir().expect("tmp");
        let (root, middle) = shared(tmp.path());
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        let foreign = commit(&a, T, "only in a");
        git_at(&a, T, &["commit-graph", "write", "--reachable"]);
        for sha in &middle {
            let loose = a.join(".git/objects").join(&sha[..2]).join(&sha[2..]);
            std::fs::remove_file(&loose).expect("loose commit object");
        }
        assert_eq!(id(&b), None, "the id came from the foreign graph");
        let mut roots = Roots::default();
        let open = Repo::discover(&b).expect("open");
        assert!(
            roots.update(&open).is_err(),
            "the roots came from the foreign graph"
        );
        let public = Repo::discover(&b).expect("open").committed_only();
        assert!(
            public.resolve(&foreign).is_err(),
            "a commit only in a is public in b"
        );
        assert!(
            public.resolve(&root).is_err(),
            "the foreign graph fed the public reachable set"
        );
    }
}
