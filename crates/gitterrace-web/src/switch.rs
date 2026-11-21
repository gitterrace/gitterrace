//! Where the repository switcher sends a visitor: the same view in the other repository when
//! that repository can have it, its root otherwise.

/// Query keys that mean the same in every repository. `rev` and filters naming paths or
/// authors are left out: they name things the other repository may not have.
const SAFE: &[&str] = &["period", "metric", "bots", "merges", "generated"];

/// The repository-relative target (path and safe query) for a page at `path` and `query`.
/// Insights, the commits log, branches and the working tree keep their section. Anything pinned
/// to a revision or a file (blob, tree, commit, diff) opens the root.
pub(crate) fn target(path: &str, query: Option<&str>) -> String {
    let path = path.trim_end_matches('/');
    let keeps_query = path == "/stats" || path.starts_with("/stats/");
    let section = if keeps_query && path.split('/').skip(1).all(plain) {
        path.to_owned()
    } else if path == "/log" || path.starts_with("/log/") {
        "/log/HEAD".to_owned()
    } else if matches!(path, "/refs" | "/worktree" | "/worktrees" | "/stashes") {
        path.to_owned()
    } else {
        return "/".to_owned();
    };
    let kept: Vec<&str> = if keeps_query {
        query
            .unwrap_or_default()
            .split('&')
            .filter(|p| SAFE.contains(&p.split('=').next().unwrap_or_default()))
            .filter(|p| p.bytes().all(|b| b.is_ascii_alphanumeric() || b"=-_.%+".contains(&b)))
            .collect()
    } else {
        Vec::new()
    };
    if kept.is_empty() { section } else { format!("{section}?{}", kept.join("&")) }
}

fn plain(seg: &str) -> bool {
    seg.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::target;

    #[test]
    fn insights_keep_page_and_safe_query() {
        assert_eq!(target("/stats/hotspots", Some("period=1m")), "/stats/hotspots?period=1m");
        assert_eq!(
            target("/stats/contributors", Some("period=3m&metric=net&bots=1&q=x&rev=dev")),
            "/stats/contributors?period=3m&metric=net&bots=1"
        );
        assert_eq!(target("/stats", None), "/stats");
    }

    #[test]
    fn log_branches_and_worktree_keep_their_section() {
        assert_eq!(target("/log/dev", Some("page=2")), "/log/HEAD");
        assert_eq!(target("/refs", None), "/refs");
        assert_eq!(target("/worktree", Some("period=1m")), "/worktree");
    }

    #[test]
    fn revision_pinned_pages_fall_back_to_the_root() {
        for p in ["/commit/abc123", "/blob/HEAD/a.rs", "/tree/main/src", "/blame/x/y", "/"] {
            assert_eq!(target(p, Some("period=1m")), "/", "{p}");
        }
        assert_eq!(target("/worktree/diff/a.rs", None), "/");
    }
}
