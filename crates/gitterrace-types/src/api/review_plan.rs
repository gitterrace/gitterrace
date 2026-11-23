//! The answer of `GET /review_plan`: the changed files in the order to review them, each with
//! the evidence for its place. Fields are in alphabetical order, as in the rest of this module.

use schemars::JsonSchema;
use serde::Serialize;

use crate::git::ChangeKind;

/// Where the changed files come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PlanSource {
    Compare,
    Worktree,
}

/// Why a file has no risk and sorts after every scored file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Lockfile,
    Generated,
    Binary,
    /// Over `limits.worktree_diff_kb` or the text limit: never read, so never scored.
    TooLarge,
}

/// Where a file ranks among the hotspots of the head tree.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PlanHotspot {
    /// Ranked files.
    pub of: usize,
    /// 1-based by hotspot score.
    pub rank: usize,
    /// `changes * sqrt(lines)`, as the hotspots page.
    pub score: f64,
}

/// One person's share of a file.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PlanOwner {
    pub name: String,
    /// 0 to 1.
    pub share: f64,
}

/// The points of `risk`, each named. `risk` is exactly their sum.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PlanTerms {
    /// 0 to 30: the hotspot rank, 30 for the hottest file.
    pub churn: u32,
    /// 0 to 25: 25 for a sole owner, 12 for a bus factor of one.
    pub ownership: u32,
    /// 0 to 25: lines added and removed against `limits.diff_fold_lines`.
    pub size: u32,
    /// 0 to 20: a source file with no test next to it.
    pub untested: u32,
}

impl PlanTerms {
    /// The sum of the terms: the one definition of `risk`.
    #[must_use]
    pub const fn sum(&self) -> u32 {
        self.churn + self.size + self.ownership + self.untested
    }
}

/// One changed file and why it sits where it does.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the contract lists each fact as its own JSON field"
)]
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PlanFile {
    pub additions: u32,
    pub binary: bool,
    /// One person alone holds half the knowledge of the file.
    pub bus_factor_one: bool,
    pub deletions: u32,
    pub generated: bool,
    pub has_test_sibling: bool,
    /// Null without a counted touch or outside the head tree.
    pub hotspot: Option<PlanHotspot>,
    pub is_test: bool,
    pub kind: ChangeKind,
    pub link: Option<String>,
    pub lockfile: bool,
    pub old_path: Option<String>,
    /// Top 3 by share.
    pub owners: Vec<PlanOwner>,
    pub path: String,
    pub permalink: Option<String>,
    /// 0 to 100, the sum of `terms`.
    pub risk: u32,
    /// Lines of the file after the change.
    pub size_lines: u32,
    /// The top owner holds at least 80 percent.
    pub sole_owner: bool,
    pub terms: PlanTerms,
}

/// A changed file that is not scored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct PlanSkipped {
    pub path: String,
    pub reason: SkipReason,
}

/// `GET /review_plan`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ReviewPlan {
    /// The commit the changes are against.
    pub base: String,
    /// Scored files, riskiest first.
    pub files: Vec<PlanFile>,
    /// Null for the working tree.
    pub head: Option<String>,
    pub since: Option<String>,
    /// Files with no risk, in path order.
    pub skipped: Vec<PlanSkipped>,
    pub source: PlanSource,
    /// Changed files in all, scored and skipped.
    pub total_files: usize,
    /// More scored files than `limit`: the rest are left out of `files`.
    pub truncated: bool,
}
