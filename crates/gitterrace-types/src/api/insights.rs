//! The answer of `GET /insights/{name}`: one shape per insight, tagged by nothing because the
//! response's `insight` names it. Fields are in alphabetical order, as in the rest of this
//! module: the JSON these replace was a `serde_json` map and its bytes must not move.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The reporting window as the summary writes it: the variant name, not the canonical text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum InsightPeriod {
    Week,
    Month,
    Quarter,
    Year,
    All,
}

/// Width of one bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum InsightUnit {
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

/// Counts over one period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightPeriodStats {
    pub additions: u64,
    pub authors: usize,
    pub commits: usize,
    pub deletions: u64,
    pub files: usize,
}

/// `summary`: the period against the one right before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightPeriodSummary {
    pub current: InsightPeriodStats,
    pub from: Option<i64>,
    pub period: InsightPeriod,
    /// Null for `All`.
    pub previous: Option<InsightPeriodStats>,
    pub to: i64,
}

/// One bucket of the activity series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightSeriesPoint {
    pub additions: u64,
    pub commits: u32,
    pub deletions: u64,
    pub key: String,
    pub label: String,
}

/// `activity`: when work happens, in each commit's own timezone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightRhythm {
    pub by_hour: [u32; 24],
    pub by_weekday: [u32; 7],
    /// `[weekday][hour]`, Monday is 0.
    pub punch_card: [[u32; 24]; 7],
    pub punch_max: u32,
    pub series: Vec<InsightSeriesPoint>,
    pub unit: InsightUnit,
}

/// One person of `contributors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightContributor {
    pub active_days: usize,
    pub additions: u64,
    pub bot: bool,
    pub commits: usize,
    pub deletions: u64,
    pub emails: Vec<String>,
    pub first_commit: i64,
    pub first_time: bool,
    pub last_commit: i64,
    pub name: String,
    /// Commits per bucket, aligned with `labels`.
    pub series: Vec<u32>,
}

/// `contributors`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightContributors {
    pub contributors: Vec<InsightContributor>,
    pub labels: Vec<String>,
    pub unit: InsightUnit,
}

/// One row of `file_churn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightFileChurn {
    pub additions: u64,
    pub commits: usize,
    pub deletions: u64,
    pub last_commit: i64,
    pub link: Option<String>,
    pub path: String,
    pub permalink: Option<String>,
}

/// One row of `dir_churn`, `.` for files at the repository root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightDirChurn {
    pub additions: u64,
    pub commits: usize,
    pub deletions: u64,
    pub files: usize,
    pub path: String,
}

/// One row of `label_breakdown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightLabelCount {
    pub commits: usize,
    pub label: String,
}

/// One bucket of `label_over_time`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightLabelBucket {
    pub counts: Vec<InsightLabelCount>,
    pub key: String,
    pub label: String,
}

/// One bucket of `frequency`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightFrequency {
    pub additions: u64,
    pub deletions: u64,
    pub key: String,
    pub label: String,
    pub rewritten: u64,
}

/// One file of `hotspots`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightHotspot {
    pub authors: usize,
    pub changes: usize,
    pub language: String,
    pub last_change: i64,
    /// Lines now.
    pub lines: u32,
    /// Added plus removed lines in the period.
    pub lines_changed: u64,
    pub link: Option<String>,
    pub path: String,
    pub permalink: Option<String>,
    /// `changes * sqrt(lines)`.
    pub score: f64,
}

/// `hotspots`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightHotspots {
    /// Share of all changes that hit the top 10% of files.
    pub concentration: f64,
    pub files: Vec<InsightHotspot>,
    pub single_author_files: usize,
    pub top_tenth: usize,
    pub total_changes: usize,
}

/// One pair of `coupling`: two files that changed in the same commits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightCouplingPair {
    pub a: String,
    pub a_changes: usize,
    pub b: String,
    pub b_changes: usize,
    /// `together / a_changes`.
    pub confidence_ab: f64,
    /// `together / b_changes`.
    pub confidence_ba: f64,
    /// The two files sit in different directories.
    pub cross_dir: bool,
    /// `together / min(a_changes, b_changes)`.
    pub degree: f64,
    /// Author time of the newest commit that changed both, unix seconds.
    pub last_together: i64,
    pub together: usize,
}

/// Commits that changed files of both top-level directories, `coupling`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightDirPair {
    pub a: String,
    pub b: String,
    pub together: usize,
}

/// `coupling`: files that change together, for one file or the whole tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightCoupling {
    /// Whether pairs across directories are listed, not only siblings.
    pub cross_dir: bool,
    /// Top-level directory pairs, empty when `path` is set.
    pub matrix: Vec<InsightDirPair>,
    /// Commits that changed more files than this are left out.
    pub max_commit_files: usize,
    /// Shared commits a pair needs.
    pub min_support: usize,
    pub pairs: Vec<InsightCouplingPair>,
    /// The focus file, null for the whole tree.
    pub path: Option<String>,
    /// Commits left out for changing more than `max_commit_files` files.
    pub skipped_commits: usize,
}

/// One person's share of a directory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightOwner {
    pub name: String,
    /// Share of the knowledge, 0 to 1.
    pub share: f64,
}

/// One directory of `ownership`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightDirOwnership {
    /// People who together hold at least half of the knowledge.
    pub bus_factor: usize,
    pub files: usize,
    pub lines: u64,
    pub owners: Vec<InsightOwner>,
    /// Empty for the whole repository.
    pub path: String,
}

/// `ownership`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightOwnership {
    pub dirs: Vec<InsightDirOwnership>,
    pub dirs_with_bus_factor_one: usize,
    pub overall: InsightDirOwnership,
    pub solo_lines: u64,
    pub stale_lines: u64,
    pub untracked_lines: u64,
}

/// One directory of `knowledge_risk`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightRiskDir {
    /// People who together hold at least half of the knowledge.
    pub bus_factor: usize,
    pub files: usize,
    /// Author time of the top owner's newest commit, unix seconds.
    pub last_active: i64,
    pub lines: u64,
    /// The top owner committed within the 12 months before the newest commit.
    pub owner_active: bool,
    pub path: String,
    /// One person holds half of it and is not active: `bus_factor` 1 and `owner_active` false.
    pub at_risk: bool,
    pub top_owner: String,
    /// The top owner's share of the directory, 0 to 1.
    pub top_share: f64,
}

/// A file of `knowledge_risk` whose main authors are all idle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightOrphan {
    /// Author time of the newest counted change, unix seconds.
    pub last_change: i64,
    pub lines: u64,
    pub path: String,
}

/// A file one person owns, `knowledge_risk`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightPersonFile {
    pub lines: u64,
    pub path: String,
    /// The person's share of the changes to the file, 0 to 1.
    pub share: f64,
}

/// What one person holds, `knowledge_risk` with `author`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightRiskPerson {
    /// Files where this person is the top owner.
    pub files: usize,
    /// Author time of the person's newest commit, unix seconds.
    pub last_active: i64,
    pub lines: u64,
    pub name: String,
    /// `lines` over the lines of every tracked file, 0 to 1.
    pub share_of_repo: f64,
    pub top_files: Vec<InsightPersonFile>,
}

/// `knowledge_risk`: who holds each directory, and the files nobody active holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InsightKnowledgeRisk {
    pub at_risk_dirs: usize,
    pub at_risk_lines: u64,
    /// The children of `path` one level down, largest first.
    pub dirs: Vec<InsightRiskDir>,
    /// Orphans under the path, all of them, not only those listed.
    pub orphan_files: usize,
    pub orphan_lines: u64,
    pub orphans: Vec<InsightOrphan>,
    /// Set when `author` is.
    pub person: Option<InsightRiskPerson>,
}

/// One language of `languages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightLanguage {
    pub files: usize,
    pub language: String,
    pub lines: u64,
}

/// `languages`: every line of the text files of the tree at `rev`, per language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InsightLanguages {
    pub files: usize,
    /// Most lines first, ties by name.
    pub languages: Vec<InsightLanguage>,
    /// The sum of `languages`, the size of the project.
    pub total_lines: u64,
}

/// The `data` of an insight answer. Which variant it is follows from `insight`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum InsightData {
    Summary(InsightPeriodSummary),
    Activity(Box<InsightRhythm>),
    Contributors(InsightContributors),
    FileChurn(Vec<InsightFileChurn>),
    DirChurn(Vec<InsightDirChurn>),
    LabelBreakdown(Vec<InsightLabelCount>),
    LabelOverTime(Vec<InsightLabelBucket>),
    Frequency(Vec<InsightFrequency>),
    Hotspots(InsightHotspots),
    Coupling(InsightCoupling),
    Ownership(InsightOwnership),
    KnowledgeRisk(InsightKnowledgeRisk),
    Languages(InsightLanguages),
}

/// `GET /insights/{name}`. The fields keep the order the answer was always written in.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct InsightResponse {
    pub commits_indexed: usize,
    /// The filter the numbers were counted under, as canonical text: `q` and the one-key
    /// parameters read into one, so a client sees how its request was understood.
    pub query: String,
    pub data: InsightData,
    pub insight: String,
    pub rev: String,
}
