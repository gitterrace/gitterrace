//! The user-visible features `[features]` switches, and the one rule that says whether one is
//! on: the file, then the repository override, then the server's policy. Off always wins; a
//! file can never turn on what `--public` or the loopback check forbids.

use crate::Config;

/// The insights pages `insights.pages` names, in their default order.
pub const INSIGHT_PAGES: [&str; 8] = [
    "activity",
    "contributors",
    "hotspots",
    "languages",
    "ownership",
    "types",
    "frequency",
    "patterns",
];

/// One switchable feature. Its key in `[features]` is [`Feature::key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Feature {
    Blame,
    Search,
    Raw,
    History,
    Compare,
    Worktree,
    Insights,
    Live,
    Actions,
    RepoAdmin,
    Picker,
    FrontMatter,
    Mermaid,
    Credit,
    Api,
}

impl Feature {
    pub const ALL: [Self; 15] = [
        Self::Blame,
        Self::Search,
        Self::Raw,
        Self::History,
        Self::Compare,
        Self::Worktree,
        Self::Insights,
        Self::Live,
        Self::Actions,
        Self::RepoAdmin,
        Self::Picker,
        Self::FrontMatter,
        Self::Mermaid,
        Self::Credit,
        Self::Api,
    ];

    /// The key under `[features]`.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Blame => "blame_enabled",
            Self::Search => "search_enabled",
            Self::Raw => "raw_enabled",
            Self::History => "history_enabled",
            Self::Compare => "compare_enabled",
            Self::Worktree => "worktree_enabled",
            Self::Insights => "insights_enabled",
            Self::Live => "live_enabled",
            Self::Actions => "actions_enabled",
            Self::RepoAdmin => "repo_admin_enabled",
            Self::Picker => "picker_enabled",
            Self::FrontMatter => "front_matter_enabled",
            Self::Mermaid => "mermaid_enabled",
            Self::Credit => "credit_enabled",
            Self::Api => "api_enabled",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.key() == key)
    }

    /// A feature of the whole server, not of one repository: `[repo.NAME.features]` refuses it.
    pub const fn is_global_only(self) -> bool {
        matches!(
            self,
            Self::RepoAdmin | Self::Picker | Self::Api | Self::Credit
        )
    }

    /// Shows this machine or changes the server, so a public server never serves it.
    pub const fn is_local_only(self) -> bool {
        matches!(
            self,
            Self::Worktree | Self::Live | Self::Actions | Self::RepoAdmin
        )
    }
}

impl Config {
    /// What `[features]` says for `f`, before any policy. `api_enabled` unset means on.
    fn file_says(&self, f: Feature) -> bool {
        let s = &self.features;
        match f {
            Feature::Blame => s.blame_enabled,
            Feature::Search => s.search_enabled,
            Feature::Raw => s.raw_enabled,
            Feature::History => s.history_enabled,
            Feature::Compare => s.compare_enabled,
            Feature::Worktree => s.worktree_enabled,
            Feature::Insights => s.insights_enabled,
            Feature::Live => s.live_enabled,
            Feature::Actions => s.actions_enabled,
            Feature::RepoAdmin => s.repo_admin_enabled,
            Feature::Picker => s.picker_enabled,
            Feature::FrontMatter => s.front_matter_enabled,
            Feature::Mermaid => s.mermaid_enabled,
            Feature::Credit => s.credit_enabled,
            // Public by choice only: a public server serves the API when the file says so.
            Feature::Api => s.api_enabled.unwrap_or(!self.server.is_public),
        }
    }

    /// Whether `f` is on for the repository named `repo` (`None`: the server as a whole).
    /// The file, the repository override and the public policy must all allow it.
    pub fn feature_on(&self, f: Feature, repo: Option<&str>) -> bool {
        if f.is_local_only() && self.server.is_public {
            return false;
        }
        let repo_off = repo
            .and_then(|r| self.repo.get(r))
            .is_some_and(|o| o.features_off.contains(&f));
        self.file_says(f) && !repo_off
    }

    /// The insights pages served for `repo`, in tab order: none when insights are off.
    pub fn insight_pages(&self, repo: Option<&str>) -> Vec<&str> {
        if !self.feature_on(Feature::Insights, repo) {
            return Vec::new();
        }
        self.insights.pages.iter().map(String::as_str).collect()
    }

    /// Whether the insights page `page` (an [`INSIGHT_PAGES`] name) is served for `repo`.
    pub fn insight_page_on(&self, page: &str, repo: Option<&str>) -> bool {
        self.insight_pages(repo).contains(&page)
    }
}

#[cfg(test)]
mod tests {
    use crate::features::{Feature, INSIGHT_PAGES};
    use crate::{keys, parse};

    /// The `[features]` keys and [`Feature`] are one list: a key added without a variant, or
    /// the other way round, fails here.
    #[test]
    fn every_features_key_is_a_feature() {
        let ks: Vec<&str> = keys()
            .iter()
            .filter(|k| k.section == "features")
            .map(|k| k.name)
            .collect();
        let fs: Vec<&str> = Feature::ALL.iter().map(|f| f.key()).collect();
        assert_eq!(ks, fs);
    }

    #[test]
    fn defaults_turn_everything_on_locally() {
        let c = crate::Config::default();
        for f in Feature::ALL {
            assert!(c.feature_on(f, None), "{f:?}");
        }
        assert_eq!(c.insight_pages(None), INSIGHT_PAGES);
    }

    /// The file can never turn on what a public server forbids, and the API is off there
    /// unless the file asks for it.
    #[test]
    fn public_policy_beats_the_file() {
        let on = Feature::ALL
            .map(|f| [f.key(), " = true\n"].concat())
            .concat();
        let text = format!("[server]\nis_public = true\n[features]\n{on}");
        let c = parse(&text).expect("parse").config;
        for f in Feature::ALL {
            assert_eq!(c.feature_on(f, None), !f.is_local_only(), "{f:?}");
        }
        let d = parse("[server]\nis_public = true\n").expect("parse").config;
        assert!(
            !d.feature_on(Feature::Api, None),
            "api is off under --public by default"
        );
        assert!(crate::Config::default().feature_on(Feature::Api, None));
    }

    /// A repository override turns off, never on, and only per-repository features.
    #[test]
    fn repo_overrides_only_turn_off() {
        let c =
            parse("[features]\nblame_enabled = false\n[repo.a.features]\nraw_enabled = false\n")
                .expect("parse")
                .config;
        assert!(!c.feature_on(Feature::Raw, Some("a")));
        assert!(c.feature_on(Feature::Raw, Some("b")));
        assert!(!c.feature_on(Feature::Blame, Some("b")));
        let e =
            parse("[features]\nblame_enabled = false\n[repo.a.features]\nblame_enabled = true\n")
                .expect_err("on is refused");
        assert!(e.0.contains("repo.a.features.blame_enabled"), "{e}");
        let e = parse("[repo.a.features]\napi_enabled = false\n").expect_err("global only");
        assert!(e.0.contains("api_enabled"), "{e}");
        let e = parse("[repo.a.features]\nblam_enabled = false\n").expect_err("unknown");
        assert!(e.0.contains("blam_enabled"), "{e}");
    }

    #[test]
    fn insight_pages_are_checked_and_ordered() {
        let e = parse("[insights]\npages = [\"activity\", \"hotspot\"]\n").expect_err("typo");
        assert!(
            e.0.contains("insights.pages") && e.0.contains("hotspot"),
            "{e}"
        );
        assert!(e.0.contains("patterns"), "names the valid ones: {e}");
        let e = parse("[insights]\npages = [\"types\", \"types\"]\n").expect_err("twice");
        assert!(e.0.contains("insights.pages"), "{e}");
        let c = parse("[insights]\npages = [\"types\", \"activity\"]\n")
            .expect("parse")
            .config;
        assert_eq!(c.insight_pages(None), ["types", "activity"]);
        let c = parse("[features]\ninsights_enabled = false\n")
            .expect("parse")
            .config;
        assert!(c.insight_pages(None).is_empty());
    }
}
