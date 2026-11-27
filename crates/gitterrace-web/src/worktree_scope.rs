//! The Staged | Unstaged | All tabs of the working tree page.

use gitterrace_git::ScopeCounts;
use gitterrace_types::WorktreeScope;

use crate::fmt;
use crate::pagehead::Tab;

/// `?scope=staged|unstaged|all`. Anything else is `all`.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ScopeQuery {
    scope: Option<String>,
    since: Option<String>,
}

impl ScopeQuery {
    pub(crate) fn scope(&self) -> WorktreeScope {
        named(self.scope.as_deref())
    }

    /// `?since=<checkpoint id>`: the page lists what differs from that checkpoint, whatever the
    /// scope says.
    pub(crate) fn since(&self) -> Option<&str> {
        self.since.as_deref().filter(|s| !s.is_empty())
    }
}

pub(crate) fn named(v: Option<&str>) -> WorktreeScope {
    v.and_then(|v| v.parse().ok()).unwrap_or_default()
}

/// The query string that selects `scope`: empty for the default.
pub(crate) const fn query(scope: WorktreeScope) -> &'static str {
    match scope {
        WorktreeScope::All => "",
        WorktreeScope::Staged => "?scope=staged",
        WorktreeScope::Unstaged => "?scope=unstaged",
    }
}

/// The query string of a page that compares with checkpoint `since` when one is named, else the
/// one `scope` selects.
pub(crate) fn query_of(scope: WorktreeScope, since: Option<&str>) -> String {
    since.map_or_else(|| query(scope).to_owned(), |id| format!("?since={}", fmt::enc_seg(id)))
}

/// The scope tabs for the page head: "Staged (n)" and so on, the current one on.
pub(crate) fn tabs(current: WorktreeScope, counts: ScopeCounts) -> Vec<Tab> {
    [
        (WorktreeScope::Staged, "Staged", counts.staged),
        (WorktreeScope::Unstaged, "Unstaged", counts.unstaged),
        (WorktreeScope::All, "All", counts.all),
    ]
    .into_iter()
    .map(|(scope, label, n)| Tab {
        label: format!("{label} ({})", fmt::thousands(n)),
        href: fmt::href(&format!("/worktree{}", query(scope))),
        on: scope == current,
        zero: n == 0,
    })
    .collect()
}

/// What an empty tab says: the whole tree is clean only on All. An empty Staged tab still
/// has unstaged work, and the reverse.
pub(crate) const fn empty_text(scope: WorktreeScope) -> &'static str {
    match scope {
        WorktreeScope::All => "The working tree is clean.",
        WorktreeScope::Staged => "Nothing is staged.",
        WorktreeScope::Unstaged => "Nothing is unstaged.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only All may call the tree clean: an empty Staged tab still has unstaged work. Catches
    /// one message shared by every scope.
    #[test]
    fn only_the_all_tab_calls_an_empty_tree_clean() {
        assert!(empty_text(WorktreeScope::All).contains("clean"));
        for s in [WorktreeScope::Staged, WorktreeScope::Unstaged] {
            assert!(!empty_text(s).contains("clean"), "{s:?}");
        }
        assert_ne!(empty_text(WorktreeScope::Staged), empty_text(WorktreeScope::Unstaged));
    }

    #[test]
    fn unknown_scope_values_fall_back_to_all() {
        for (raw, want) in [
            (Some("staged"), WorktreeScope::Staged),
            (Some("unstaged"), WorktreeScope::Unstaged),
            (Some("bogus"), WorktreeScope::All),
            (None, WorktreeScope::All),
        ] {
            let q = ScopeQuery { scope: raw.map(str::to_owned), since: None };
            assert_eq!(q.scope(), want);
        }
    }

    #[test]
    fn tabs_mark_one_current_and_carry_counts() {
        let t = tabs(WorktreeScope::Staged, ScopeCounts { all: 3, staged: 2, unstaged: 2 });
        assert_eq!(t.iter().filter(|t| t.on).count(), 1);
        assert!(t[0].on && t[0].label == "Staged (2)");
        assert!(t[0].href.ends_with("/worktree?scope=staged"));
        assert!(t[2].href.ends_with("/worktree"));
    }
}
