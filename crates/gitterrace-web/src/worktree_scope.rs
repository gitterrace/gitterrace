//! The Staged | Unstaged | All tabs of the working tree page.

use gitterrace_git::{Scope, ScopeCounts};

use crate::fmt;

/// `?scope=staged|unstaged|all`. Anything else is `all`.
#[derive(Debug, Default, serde::Deserialize)]
pub(crate) struct ScopeQuery {
    scope: Option<String>,
}

impl ScopeQuery {
    pub(crate) fn scope(&self) -> Scope {
        match self.scope.as_deref() {
            Some("staged") => Scope::Staged,
            Some("unstaged") => Scope::Unstaged,
            _ => Scope::All,
        }
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

pub(crate) struct Tab {
    pub(crate) label: &'static str,
    pub(crate) href: String,
    pub(crate) count: String,
    pub(crate) current: bool,
}

pub(crate) fn tabs(current: Scope, counts: ScopeCounts) -> Vec<Tab> {
    [
        (Scope::Staged, "Staged", counts.staged),
        (Scope::Unstaged, "Unstaged", counts.unstaged),
        (Scope::All, "All", counts.all),
    ]
    .into_iter()
    .map(|(scope, label, n)| Tab {
        label,
        href: fmt::href(&format!("/worktree{}", query(scope))),
        count: fmt::thousands(n),
        current: scope == current,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_scope_values_fall_back_to_all() {
        for (raw, want) in [
            (Some("staged"), Scope::Staged),
            (Some("unstaged"), Scope::Unstaged),
            (Some("bogus"), Scope::All),
            (None, Scope::All),
        ] {
            let q = ScopeQuery {
                scope: raw.map(str::to_owned),
            };
            assert_eq!(q.scope(), want);
        }
    }

    #[test]
    fn tabs_mark_one_current_and_carry_counts() {
        let t = tabs(
            Scope::Staged,
            ScopeCounts {
                all: 3,
                staged: 2,
                unstaged: 2,
            },
        );
        assert_eq!(t.iter().filter(|t| t.current).count(), 1);
        assert!(t[0].current && t[0].count == "2");
        assert!(t[0].href.ends_with("/worktree?scope=staged"));
        assert!(t[2].href.ends_with("/worktree"));
    }
}
