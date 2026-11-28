//! Which `[features]` and insights page each MCP tool needs, the same as its web route.

use gitterrace_config::{Config, Feature};

/// What a tool needs on: the features and the insights page the web's `/api/v1` route of the
/// same answer needs. `api_enabled` is needed by every tool. A tool missing here is refused.
const TOOL_NEEDS: &[(&str, &[Feature], Option<&str>)] = &[
    ("list_repos", &[], None),
    ("repo_overview", &[], None),
    ("log", &[], None),
    ("show_commit", &[], None),
    ("compare", &[Feature::Compare], None),
    ("worktree_diff", &[Feature::Worktree], None),
    ("list_checkpoints", &[Feature::Worktree], None),
    ("work_state", &[Feature::Worktree], None),
    ("list_viewed", &[Feature::Worktree], None),
    ("list_findings", &[Feature::Worktree], None),
    ("read_file", &[], None),
    ("resolve_permalink", &[], None),
    ("list_tree", &[], None),
    ("blame", &[Feature::Blame], None),
    ("contributors", &[Feature::Insights], Some("contributors")),
    ("file_insight", &[Feature::Insights], None),
    ("review_plan", &[Feature::Insights], None),
    ("insight", &[Feature::Insights], None),
    ("list_insights", &[Feature::Insights], None),
];

/// Whether MCP tool `tool` is served for `repo` under `cfg`: `[features]` (`api_enabled`
/// included), the repository's overrides and the insights pages decide, as for the web API.
pub fn tool_on(cfg: &Config, repo: Option<&str>, tool: &str) -> bool {
    let Some((_, features, page)) = TOOL_NEEDS.iter().find(|(n, ..)| *n == tool) else {
        return false;
    };
    cfg.feature_on(Feature::Api, repo)
        && features.iter().all(|f| cfg.feature_on(*f, repo))
        && page.is_none_or(|p| cfg.insight_page_on(p, repo))
}
