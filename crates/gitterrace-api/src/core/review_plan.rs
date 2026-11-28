//! The review plan: the changed files of a compare or the working tree, scored and ordered,
//! see [`ApiState::review_plan`].

mod chapters;

use std::collections::BTreeSet;

use gitterrace_git::FileDiff;
use gitterrace_index::FileFacts;
use gitterrace_types::api::{
    PlanFile, PlanHotspot, PlanOwner, PlanSkipped, PlanSource, PlanTerms, ReviewPlan, SkipReason,
};

use crate::bounds::checked;
use crate::core::ApiState;
use crate::error::ApiError;
use crate::insights;

/// Points of the churn term at hotspot rank 1. Not configurable: the weights of `risk` are code
/// so that a score means the same on every machine. They sum to 100.
const CHURN_POINTS: f64 = 30.0;
/// Points of the size term at `limits.diff_fold_lines` changed lines.
const SIZE_POINTS: f64 = 25.0;
/// Points of the ownership term for a sole owner.
const SOLE_POINTS: u32 = 25;
/// Points of the ownership term for a bus factor of one without a sole owner.
const BUS_ONE_POINTS: u32 = 12;
/// Points of the untested term.
const UNTESTED_POINTS: u32 = 20;

/// The terms of `risk` with what each earns, for the docs: `(name, rule)`.
#[must_use]
pub fn risk_terms() -> [(&'static str, String); 4] {
    [
        ("churn", format!("up to {CHURN_POINTS}, the hotspot rank")),
        ("size", format!("up to {SIZE_POINTS}, changed lines against `limits.diff_fold_lines`")),
        (
            "ownership",
            format!("{SOLE_POINTS} for a sole owner, {BUS_ONE_POINTS} for a bus factor of one"),
        ),
        ("untested", format!("{UNTESTED_POINTS} for a source file with no test beside it")),
    ]
}

/// The facts of a file whose history could not be read: no hotspot, no owners.
const fn no_facts() -> FileFacts {
    FileFacts { hotspot: None, owners: Vec::new(), bus_factor_one: false, sole_owner: false }
}

/// Folders whose files are tests themselves.
const TEST_DIRS: [&str; 4] = ["test", "tests", "__tests__", "spec"];

/// What a review plan reads: the source of the changed files and its arguments.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlanRequest<'a> {
    /// `compare` or `worktree` (default).
    pub source: Option<&'a str>,
    /// Compare: the revision to compare from.
    pub base: Option<&'a str>,
    /// Compare: the revision to compare to, default `HEAD`.
    pub head: Option<&'a str>,
    /// Compare: from the merge base.
    pub three_dot: Option<bool>,
    /// Worktree: `all` (default), `staged` or `unstaged`.
    pub scope: Option<&'a str>,
    /// Worktree: a checkpoint id, only what changed since it.
    pub since: Option<&'a str>,
}

/// One changed file before scoring.
#[expect(clippy::struct_excessive_bools, reason = "each fact is its own field of the answer")]
struct Changed<'a> {
    diff: &'a FileDiff,
    skip: Option<SkipReason>,
    generated: bool,
    lockfile: bool,
    is_test: bool,
    has_sibling: bool,
}

/// The changed files a plan scores and what they are against.
struct Source {
    kind: PlanSource,
    base: String,
    head: Option<String>,
    rev: String,
    list: Vec<FileDiff>,
    since: Option<String>,
    baseline_missing: bool,
    history_unreadable: bool,
    content_gone: Vec<String>,
    files_omitted: bool,
}

impl ApiState {
    /// The changed files of a compare or of the working tree in the order to review them: most
    /// risk first, each with the numbers behind its `risk`. Counted as the `hotspots` and
    /// `ownership` reports count under `params`. The working tree is this machine's, so a
    /// committed-only (public) handle and a repository with `worktree_enabled` off refuse it.
    /// `params.limit` cuts the answer, at most `review.plan_files`.
    pub fn review_plan(
        &self,
        req: &PlanRequest<'_>,
        params: &insights::Params,
    ) -> Result<ReviewPlan, ApiError> {
        let settings = self.settings();
        let most = settings.review.plan_files;
        let limit = params.limit.map_or(most, |l| l.clamp(1, most));
        let Source {
            kind: source,
            base,
            head,
            rev,
            list,
            since,
            baseline_missing,
            history_unreadable,
            content_gone,
            files_omitted,
        } = self.plan_source(req)?;
        let tip = self.repo.resolve(&rev)?;
        let heads = self.head_lines(&tip);
        let tree: BTreeSet<&str> = heads
            .iter()
            .map(|f| f.path.as_str())
            .chain(
                list.iter()
                    .filter(|f| f.kind != gitterrace_types::ChangeKind::Deleted)
                    .map(|f| f.path.as_str()),
            )
            .collect();
        let generated = gitterrace_git::generated_paths(
            &self.repo,
            &tip,
            list.iter().map(|f| f.path.as_str()),
        )?;
        let patterns = &settings.review.test_siblings;
        let changed: Vec<Changed<'_>> = list
            .iter()
            .map(|diff| {
                let lockfile = gitterrace_git::is_lockfile(&diff.path);
                let generated = generated.contains(&diff.path);
                Changed {
                    diff,
                    skip: self.skip_reason(diff, &tip, lockfile, generated),
                    generated,
                    lockfile,
                    is_test: is_test(&diff.path, patterns),
                    has_sibling: has_sibling(&diff.path, patterns, &tree),
                }
            })
            .collect();
        let scored: Vec<&Changed<'_>> = changed.iter().filter(|c| c.skip.is_none()).collect();
        let paths: Vec<&str> = scored.iter().map(|c| c.diff.path.as_str()).collect();
        let scope = insights::scope(params, settings.defaults.clone())?;
        let facts = self.read(&rev, |all| {
            let ins = self.insights(&tip, all, scope.options().clone());
            let counted = insights::counted(&ins, &scope, &**all);
            ins.file_facts(&heads, &counted, &paths)
        });
        let facts = match facts {
            Ok(f) => f,
            // Unreadable history is why the cache cannot record the commits either: the plan
            // still lists the files, unscored by history, and says so through the flag.
            Err(_) if history_unreadable => paths.iter().map(|_| no_facts()).collect(),
            Err(e) => return Err(e),
        };
        let mut files: Vec<PlanFile> = scored
            .iter()
            .zip(facts)
            .map(|(c, f)| {
                let mut file = plan_file(c, f, settings.fold_lines);
                if source == PlanSource::Compare
                    && c.diff.kind != gitterrace_types::ChangeKind::Deleted
                {
                    (file.permalink, file.link) = self.links(&tip, Some(&file.path), None);
                }
                file
            })
            .collect();
        order(&mut files);
        mark_signals(&mut files, &list, patterns);
        let mut skipped: Vec<PlanSkipped> = changed
            .iter()
            .filter_map(|c| c.skip.map(|reason| PlanSkipped { path: c.diff.path.clone(), reason }))
            .collect();
        skipped.sort_by(|a, b| a.path.cmp(&b.path));
        let total_files = files.len() + skipped.len();
        let truncated = files.len() > limit;
        files.truncate(limit);
        let chapters = chapters::chapters(&files, &skipped);
        Ok(ReviewPlan {
            base,
            baseline_missing,
            chapters,
            content_gone,
            files,
            files_omitted,
            head,
            history_unreadable,
            since,
            skipped,
            source,
            total_files,
            truncated,
        })
    }

    /// The changed files `req` names and where they are against.
    fn plan_source(&self, req: &PlanRequest<'_>) -> Result<Source, ApiError> {
        match req.source.unwrap_or("worktree") {
            "worktree" => {
                if !self.settings().worktree_on {
                    return Err(ApiError::Forbidden("the working tree"));
                }
                let w = self.worktree_list(req.scope, req.since)?;
                let mut base = self.repo.resolve("HEAD")?;
                let mut baseline_missing = false;
                let mut history_unreadable = false;
                let mut content_gone = Vec::new();
                if let Some((_, cp_head, d)) = &w.since {
                    baseline_missing = d.baseline_missing;
                    history_unreadable = d.history_unreadable;
                    content_gone.clone_from(&d.gone);
                    // The comparison is against the checkpoint's commit; with it missing, the
                    // base stays the current HEAD, which `base` already is.
                    if !baseline_missing {
                        base = self.repo.resolve(cp_head).unwrap_or(base);
                    }
                }
                Ok(Source {
                    kind: PlanSource::Worktree,
                    base,
                    head: None,
                    rev: "HEAD".to_owned(),
                    list: w.files,
                    since: w.since.map(|s| s.0),
                    baseline_missing,
                    history_unreadable,
                    content_gone,
                    files_omitted: w.files_omitted,
                })
            }
            "compare" => {
                let base = req
                    .base
                    .filter(|b| !b.is_empty())
                    .ok_or_else(|| ApiError::BadRequest("source compare needs base".to_owned()))?;
                let head = checked(req.head.filter(|h| !h.is_empty()).unwrap_or("HEAD"))?;
                let (from, list) = self.repo.compare_files(
                    checked(base)?,
                    head,
                    req.three_dot.unwrap_or(false),
                )?;
                let tip = self.repo.resolve(head)?;
                Ok(Source {
                    kind: PlanSource::Compare,
                    base: from,
                    head: Some(tip.clone()),
                    rev: tip,
                    list,
                    since: None,
                    baseline_missing: false,
                    history_unreadable: false,
                    content_gone: Vec::new(),
                    files_omitted: false,
                })
            }
            other => Err(ApiError::BadRequest(format!(
                "unknown source {other:?}: use `compare` or `worktree`"
            ))),
        }
    }

    /// Why `diff` is not scored. Git reports a file it never read for size as binary, so a
    /// binary file over the text limit at the head is told apart as `too_large`.
    fn skip_reason(
        &self,
        diff: &FileDiff,
        tip: &str,
        lockfile: bool,
        generated: bool,
    ) -> Option<SkipReason> {
        if lockfile {
            Some(SkipReason::Lockfile)
        } else if generated {
            Some(SkipReason::Generated)
        } else if diff.binary {
            let big =
                self.repo.blob(tip, &diff.path).is_ok_and(|b| b.size > self.repo.text_limit());
            Some(if big { SkipReason::TooLarge } else { SkipReason::Binary })
        } else {
            None
        }
    }
}

/// The scored row of one changed file.
fn plan_file(c: &Changed<'_>, f: FileFacts, fold_lines: usize) -> PlanFile {
    let d = c.diff;
    let terms = terms(&Inputs {
        changed_lines: u64::from(d.additions) + u64::from(d.deletions),
        rank: f.hotspot.map(|(rank, of, _)| (rank, of)),
        sole_owner: f.sole_owner,
        bus_factor_one: f.bus_factor_one,
        untested: !c.is_test && !c.has_sibling && !c.generated && !c.lockfile,
        fold_lines,
    });
    PlanFile {
        additions: d.additions,
        binary: d.binary,
        bus_factor_one: f.bus_factor_one,
        deletions: d.deletions,
        generated: c.generated,
        has_test_sibling: c.has_sibling,
        hotspot: f.hotspot.map(|(rank, of, score)| PlanHotspot { of, rank, score }),
        is_test: c.is_test,
        kind: d.kind,
        link: None,
        lockfile: c.lockfile,
        old_path: d.old_path.clone(),
        owners: f.owners.into_iter().map(|o| PlanOwner { name: o.name, share: o.share }).collect(),
        path: d.path.clone(),
        permalink: None,
        risk: terms.sum(),
        signals: Vec::new(),
        size_lines: d.lines,
        sole_owner: f.sole_owner,
        terms,
    }
}

/// What the terms of one file are computed from.
struct Inputs {
    changed_lines: u64,
    /// `(rank, ranked files)`.
    rank: Option<(usize, usize)>,
    sole_owner: bool,
    bus_factor_one: bool,
    untested: bool,
    /// `limits.diff_fold_lines`.
    fold_lines: usize,
}

/// The four terms. Each is rounded once, on its own, so `risk` is their exact sum.
fn terms(i: &Inputs) -> PlanTerms {
    let churn = i.rank.map_or(0.0, |(rank, of)| {
        CHURN_POINTS * (1.0 - rank.saturating_sub(1) as f64 / of.max(1) as f64)
    });
    let size = (SIZE_POINTS * i.changed_lines as f64 / i.fold_lines.max(1) as f64).min(SIZE_POINTS);
    let ownership = if i.sole_owner {
        SOLE_POINTS
    } else if i.bus_factor_one {
        BUS_ONE_POINTS
    } else {
        0
    };
    PlanTerms {
        churn: churn.round() as u32,
        ownership,
        size: size.round() as u32,
        untested: if i.untested { UNTESTED_POINTS } else { 0 },
    }
}

/// Each file's risk signals, a test sibling counted as touched when it is in the change too.
fn mark_signals(files: &mut [PlanFile], list: &[FileDiff], patterns: &[String]) {
    let touched: BTreeSet<&str> = list.iter().map(|f| f.path.as_str()).collect();
    for file in files {
        let sibling_touched = has_sibling(&file.path, patterns, &touched);
        file.signals = chapters::signals(file, sibling_touched);
    }
}

/// Riskiest first, then the bigger change, then the path: the same input, the same order.
fn order(files: &mut [PlanFile]) {
    files.sort_by(|a, b| {
        b.risk
            .cmp(&a.risk)
            .then_with(|| (b.additions + b.deletions).cmp(&(a.additions + a.deletions)))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Splits `path` into folder, name without extension and extension.
fn parts(path: &str) -> (&str, &str, &str) {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (dir, stem, ext)
}

/// Whether the file is a test: it matches a pattern read from its right end (the placeholders
/// stand for anything), or sits under a test folder.
fn is_test(path: &str, patterns: &[String]) -> bool {
    let folders: Vec<&str> = path.split('/').rev().skip(1).collect();
    if folders.iter().any(|f| TEST_DIRS.contains(f)) {
        return true;
    }
    let starts: Vec<usize> =
        std::iter::once(0).chain(path.match_indices('/').map(|(i, _)| i + 1)).collect();
    patterns.iter().any(|p| {
        let open = p.replace("{dir}/", "").replace("{stem}", "*").replace("{ext}", "*");
        starts.iter().any(|&s| glob(&open, &path[s..]))
    })
}

/// Whether a file of `tree` (the head tree and the changed files) is a test of `path`.
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the placeholders are the config's own, not format arguments"
)]
fn has_sibling(path: &str, patterns: &[String], tree: &BTreeSet<&str>) -> bool {
    let (dir, stem, ext) = parts(path);
    patterns.iter().any(|p| {
        let mut s = p.replace("{stem}", stem).replace("{ext}", ext);
        s = if dir.is_empty() { s.replace("{dir}/", "") } else { s.replace("{dir}", dir) };
        if !s.contains('*') {
            return s != path && tree.contains(s.as_str());
        }
        let fixed = s.split('*').next().unwrap_or_default();
        tree.range(fixed..).take_while(|t| t.starts_with(fixed)).any(|t| *t != path && glob(&s, t))
    })
}

/// Glob match: `*` is any run inside one folder name, `**` any run across folders, and `**/`
/// may match no folder at all.
fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some(b'*') if p.starts_with(b"**/") => {
                go(&p[3..], t) || (0..t.len()).any(|i| t[i] == b'/' && go(&p[3..], &t[i + 1..]))
            }
            Some(b'*') if p.starts_with(b"**") => (0..=t.len()).any(|i| go(&p[2..], &t[i..])),
            Some(b'*') => {
                let run = t.iter().position(|c| *c == b'/').unwrap_or(t.len());
                (0..=run).any(|i| go(&p[1..], &t[i..]))
            }
            Some(c) => t.first() == Some(c) && go(&p[1..], &t[1..]),
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests;
