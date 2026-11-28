//! Chapters and risk signals of a review plan: pure functions of the scored rows, so the same
//! plan always reads in the same order. A wrong grouping is only a worse order, never a wrong
//! fact, and a signal names its evidence instead of moving `risk`.

use std::collections::BTreeMap;

use gitterrace_types::ChangeKind;
use gitterrace_types::api::{
    ChapterKind, PlanChapter, PlanFile, PlanSignal, PlanSkipped, SignalKind,
};

/// Removed lines from which a deletion is large.
const LARGE_DELETION_LINES: u32 = 100;
/// A deletion is large only when it removes at least this many times what it adds.
const LARGE_DELETION_RATIO: u32 = 3;
/// Folders that hold one package per subfolder: the package is the chapter, not the folder.
const PACKAGE_HOLDERS: [&str; 5] = ["crates", "packages", "apps", "libs", "services"];
/// File names that are manifests or build configuration.
const CONFIG_NAMES: [&str; 8] = [
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "Makefile",
    "Dockerfile",
    "tsconfig.json",
    "build.rs",
];
/// Folders whose files configure the build or CI.
const CONFIG_DIRS: [&str; 3] = [".github/", ".gitlab/", ".circleci/"];
/// The title of the chapter of unscored files.
const MECHANICAL_TITLE: &str = "Not scored";
/// The title of the chapter of files at the top of the tree.
const ROOT_TITLE: &str = "Top level";

/// What looks off about `file`, each with its evidence. `sibling_touched`: a test next to the
/// file is among the changed files too.
pub(super) fn signals(file: &PlanFile, sibling_touched: bool) -> Vec<PlanSignal> {
    let mut out = Vec::new();
    if file.is_test && file.kind == ChangeKind::Deleted {
        out.push(PlanSignal {
            kind: SignalKind::TestRemoved,
            evidence: format!("{} is deleted", file.path),
        });
    }
    if !file.is_test && file.has_test_sibling && !sibling_touched {
        out.push(PlanSignal {
            kind: SignalKind::SourceWithoutTestChange,
            evidence: "it has a test next to it and that test is not in the change".to_owned(),
        });
    }
    if is_config(&file.path) {
        out.push(PlanSignal {
            kind: SignalKind::ConfigChange,
            evidence: format!("{} configures the build or CI", file.path),
        });
    }
    if file.deletions >= LARGE_DELETION_LINES
        && file.deletions >= file.additions.saturating_mul(LARGE_DELETION_RATIO)
    {
        out.push(PlanSignal {
            kind: SignalKind::LargeDeletion,
            evidence: format!("{} lines removed, {} added", file.deletions, file.additions),
        });
    }
    out
}

/// Whether `path` is a manifest, a build file or a CI file.
fn is_config(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    CONFIG_NAMES.contains(&name) || CONFIG_DIRS.iter().any(|d| path.starts_with(d))
}

/// The chapter a path belongs to: its package folder, else its top folder.
fn root(path: &str) -> String {
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or_default();
    let Some(second) = parts.next() else {
        return ROOT_TITLE.to_owned();
    };
    // A path with only two parts has its file name second, so the folder is the root.
    if PACKAGE_HOLDERS.contains(&first) && parts.next().is_some() {
        format!("{first}/{second}")
    } else {
        first.to_owned()
    }
}

/// Docs are read after code.
fn is_doc(path: &str) -> bool {
    path.starts_with("docs/")
        || [".md", ".rst", ".txt"].iter().any(|e| path.to_ascii_lowercase().ends_with(e))
}

/// One folder's rows while chapters are built.
struct Draft<'a> {
    files: Vec<&'a PlanFile>,
}

/// Groups `files` (riskiest first) by folder root, the highest risk first, with chapters of
/// only tests or only docs after the code ones, and the unscored `skipped` files last.
pub(super) fn chapters(files: &[PlanFile], skipped: &[PlanSkipped]) -> Vec<PlanChapter> {
    let mut drafts: BTreeMap<String, Draft<'_>> = BTreeMap::new();
    for f in files {
        drafts.entry(root(&f.path)).or_insert_with(|| Draft { files: Vec::new() }).files.push(f);
    }
    let mut out: Vec<(bool, PlanChapter)> = drafts
        .into_iter()
        .map(|(title, d)| {
            let tail = d.files.iter().all(|f| f.is_test || is_doc(&f.path));
            let chapter = PlanChapter {
                id: format!("dir:{title}"),
                kind: ChapterKind::Dir,
                risk_max: d.files.iter().map(|f| f.risk).max().unwrap_or(0),
                risk_sum: d.files.iter().map(|f| f.risk).sum(),
                files: d.files.iter().map(|f| f.path.clone()).collect(),
                title,
            };
            (tail, chapter)
        })
        .collect();
    out.sort_by(|(ta, a), (tb, b)| {
        ta.cmp(tb)
            .then_with(|| b.risk_max.cmp(&a.risk_max))
            .then_with(|| b.risk_sum.cmp(&a.risk_sum))
            .then_with(|| a.title.cmp(&b.title))
    });
    let mut out: Vec<PlanChapter> = out.into_iter().map(|(_, c)| c).collect();
    if !skipped.is_empty() {
        out.push(PlanChapter {
            id: "mechanical".to_owned(),
            kind: ChapterKind::Mechanical,
            risk_max: 0,
            risk_sum: 0,
            title: MECHANICAL_TITLE.to_owned(),
            files: skipped.iter().map(|s| s.path.clone()).collect(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use gitterrace_types::api::{PlanTerms, SkipReason};

    use super::*;

    fn file(path: &str, risk: u32) -> PlanFile {
        PlanFile {
            additions: 1,
            binary: false,
            bus_factor_one: false,
            deletions: 0,
            generated: false,
            has_test_sibling: false,
            hotspot: None,
            is_test: false,
            kind: ChangeKind::Modified,
            link: None,
            lockfile: false,
            old_path: None,
            owners: Vec::new(),
            path: path.to_owned(),
            permalink: None,
            risk,
            signals: Vec::new(),
            size_lines: 1,
            sole_owner: false,
            terms: PlanTerms::default(),
        }
    }

    fn kinds(f: &PlanFile, sibling_touched: bool) -> Vec<SignalKind> {
        signals(f, sibling_touched).into_iter().map(|s| s.kind).collect()
    }

    #[test]
    fn a_deleted_test_is_a_removed_test_and_a_kept_one_is_not() {
        let mut t = file("tests/a.rs", 0);
        t.is_test = true;
        t.kind = ChangeKind::Deleted;
        assert_eq!(kinds(&t, false), [SignalKind::TestRemoved]);
        t.kind = ChangeKind::Modified;
        assert_eq!(kinds(&t, false), []);
    }

    #[test]
    fn a_source_change_flags_a_missing_test_change_only_when_a_test_exists() {
        let mut f = file("src/a.rs", 0);
        assert_eq!(kinds(&f, false), [], "no test beside it is the untested term");
        f.has_test_sibling = true;
        assert_eq!(kinds(&f, false), [SignalKind::SourceWithoutTestChange]);
        assert_eq!(kinds(&f, true), [], "its test changed too");
    }

    #[test]
    fn manifests_and_ci_files_are_config_changes_and_source_is_not() {
        for p in ["Cargo.toml", "crates/x/Cargo.toml", ".github/workflows/ci.yml"] {
            assert_eq!(kinds(&file(p, 0), false), [SignalKind::ConfigChange], "{p}");
        }
        assert_eq!(kinds(&file("src/config.rs", 0), false), []);
    }

    #[test]
    fn a_deletion_is_large_by_size_and_by_ratio() {
        let mut f = file("src/a.rs", 0);
        f.deletions = 100;
        f.additions = 10;
        assert_eq!(kinds(&f, false), [SignalKind::LargeDeletion]);
        f.additions = 40;
        assert_eq!(kinds(&f, false), [], "a rewrite is not a deletion");
        f.deletions = 99;
        f.additions = 0;
        assert_eq!(kinds(&f, false), [], "under the size floor");
    }

    #[test]
    fn chapters_group_by_package_and_order_code_before_tests_and_docs() {
        let mut t = file("crates/a/tests/t.rs", 90);
        t.is_test = true;
        let files = [
            t,
            file("docs/x.md", 80),
            file("crates/b/src/lib.rs", 50),
            file("crates/b/src/mod.rs", 60),
            file("crates/a/src/lib.rs", 20),
            file("README", 5),
        ];
        let skipped = [PlanSkipped { path: "Cargo.lock".to_owned(), reason: SkipReason::Lockfile }];
        let c = chapters(&files, &skipped);
        let ids: Vec<&str> = c.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            ["dir:crates/a", "dir:crates/b", "dir:Top level", "dir:docs", "mechanical"]
        );
        assert_eq!(c[1].files, ["crates/b/src/lib.rs", "crates/b/src/mod.rs"]);
        assert_eq!((c[1].risk_max, c[1].risk_sum), (60, 110));
        assert_eq!(c[4].kind, ChapterKind::Mechanical);
        assert_eq!(c[4].files, ["Cargo.lock"]);
    }

    #[test]
    fn no_unscored_files_means_no_mechanical_chapter() {
        let c = chapters(&[file("a/b.rs", 1)], &[]);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, ChapterKind::Dir);
    }
}
