//! The commit-graph a full-history walk reads parents from, so it decodes no commit object.

/// The repository's commit-graph, or else the first one an alternate object store holds.
///
/// gix looks only in the repository's own `objects/info`, so a repository that borrows its
/// objects from another (`git clone --shared`, an alternates file) walks without a graph and
/// decodes every commit. A graph indexes commits by id, so one written in the alternate serves
/// the commits it holds; the walk reads the rest from objects. `None` when the configuration
/// turns the graph off, and for a shallow repository: the alternate's graph records the parents
/// the shallow boundary cuts off, so it would change what the walk reaches.
pub(crate) fn commit_graph(repo: &gix::Repository) -> Option<gix::commitgraph::Graph> {
    if let Ok(Some(graph)) = repo.commit_graph_if_enabled() {
        return Some(graph);
    }
    let disabled = repo.config_snapshot().boolean("core.commitGraph") == Some(false);
    if disabled || repo.is_shallow().unwrap_or(true) {
        return None;
    }
    let alternates = repo.objects.store_ref().alternate_db_paths().ok()?;
    alternates.iter().find_map(|objects| {
        gix::commitgraph::Graph::at(&objects.join("info"), repo.object_hash(), None).ok()
    })
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

    /// Counted walk: the clone borrows every object from an alternate that holds a graph,
    /// and the middle commits' objects are deleted. The id and the roots must come from the
    /// graph alone, decoding only the parentless commit; a walk that decodes one commit
    /// object per history commit fails on the first missing one.
    #[test]
    fn id_and_roots_read_no_history_commit_object() {
        let tmp = tempfile::tempdir().expect("tmp");
        let a = tmp.path().join("a");
        std::fs::create_dir(&a).expect("dir");
        git_at(&a, T, &["init", "-q", "-b", "main"]);
        git_at(&a, T, &["config", "user.name", "T"]);
        git_at(&a, T, &["config", "user.email", "t@example.com"]);
        let root = commit(&a, T, "root");
        let middle: Vec<String> = (0..3).map(|i| commit(&a, T, &format!("c{i}"))).collect();
        commit(&a, T, "tip");
        git_at(&a, T, &["commit-graph", "write", "--reachable"]);
        let b = tmp.path().join("b");
        let (from, to) = (a.to_string_lossy(), b.to_string_lossy());
        git_at(tmp.path(), T, &["clone", "-q", "--shared", &from, &to]);
        for sha in &middle {
            let loose = a.join(".git/objects").join(&sha[..2]).join(&sha[2..]);
            std::fs::remove_file(&loose).expect("loose commit object");
        }
        assert_eq!(id(&b), Some(root[..12].to_owned()));
        let mut roots = Roots::default();
        roots
            .update(&Repo::discover(&b).expect("open"))
            .expect("roots");
        assert_eq!(roots.ids().iter().collect::<Vec<_>>(), [&root]);
    }
}
