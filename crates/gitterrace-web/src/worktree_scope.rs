//! The Staged | Unstaged | All tabs of the working tree page.

use gitterrace_git::{Scope, ScopeCounts};

use crate::fmt;
use crate::pagehead::Tab;

/// `?scope=staged|unstaged|all`. Anything else is `all`.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ScopeQuery {
    scope: Option<String>,
}

impl ScopeQuery {
    pub(crate) fn scope(&self) -> Scope {
        named(self.scope.as_deref())
    }
}

pub(crate) fn named(v: Option<&str>) -> Scope {
    match v {
        Some("staged") => Scope::Staged,
        Some("unstaged") => Scope::Unstaged,
        _ => Scope::All,
    }
}

/// The query string that selects `scope`: empty for the default.
pub(crate) const fn query(scope: Scope) -> &'static str {
    match scope {
        Scope::All => "",
        Scope::Staged => "?scope=staged",
        Scope::Unstaged => "?scope=unstaged",
    }
}

/// The scope tabs for the page head: "Staged (n)" and so on, the current one on.
pub(crate) fn tabs(current: Scope, counts: ScopeCounts) -> Vec<Tab> {
    [
        (Scope::Staged, "Staged", counts.staged),
        (Scope::Unstaged, "Unstaged", counts.unstaged),
        (Scope::All, "All", counts.all),
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
pub(crate) const fn empty_text(scope: Scope) -> &'static str {
    match scope {
        Scope::All => "The working tree is clean.",
        Scope::Staged => "Nothing is staged.",
        Scope::Unstaged => "Nothing is unstaged.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only All may call the tree clean: an empty Staged tab still has unstaged work. Catches
    /// one message shared by every scope.
    #[test]
    fn only_the_all_tab_calls_an_empty_tree_clean() {
        assert!(empty_text(Scope::All).contains("clean"));
        for s in [Scope::Staged, Scope::Unstaged] {
            assert!(!empty_text(s).contains("clean"), "{s:?}");
        }
        assert_ne!(empty_text(Scope::Staged), empty_text(Scope::Unstaged));
    }

    #[test]
    fn unknown_scope_values_fall_back_to_all() {
        for (raw, want) in [
            (Some("staged"), Scope::Staged),
            (Some("unstaged"), Scope::Unstaged),
            (Some("bogus"), Scope::All),
            (None, Scope::All),
        ] {
            let q = ScopeQuery { scope: raw.map(str::to_owned) };
            assert_eq!(q.scope(), want);
        }
    }

    #[test]
    fn tabs_mark_one_current_and_carry_counts() {
        let t = tabs(Scope::Staged, ScopeCounts { all: 3, staged: 2, unstaged: 2 });
        assert_eq!(t.iter().filter(|t| t.on).count(), 1);
        assert!(t[0].on && t[0].label == "Staged (2)");
        assert!(t[0].href.ends_with("/worktree?scope=staged"));
        assert!(t[2].href.ends_with("/worktree"));
    }
}
