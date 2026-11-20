//! The compare page, `/compare`: what changed between two revisions. It belongs to Commits: the
//! top nav marks Commits, the log links here, and an empty form starts from the latest tag.
//! Every link to the page comes from [`compare_href`], so the prefill and the feature switch
//! hold on all of them.

use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use gitterrace_index::{Comparison, RawCommit};
use serde::Deserialize;

use crate::{
    app::App,
    empty::Empty,
    error::Result,
    fmt::{self, enc_seg, href},
    insights::{Kpi, Src, cached, insights},
    pagehead::{Action, PageHead, Tagline},
    routes::Shared,
    views::{self, Chrome},
};

#[derive(Deserialize, Default)]
pub(crate) struct CompareQuery {
    base: Option<String>,
    head: Option<String>,
    /// `merge-base` compares from where `head` left `base` (`base...head`). Anything else is
    /// the plain range `base..head`.
    mode: Option<String>,
}

/// The query value of three-dot mode.
const MERGE_BASE: &str = "merge-base";

struct LangRow {
    language: String,
    added: String,
    removed: String,
    net: String,
    tone: &'static str,
}

struct Result2 {
    /// Set when the range is empty: what that means and the way on.
    empty: Option<Empty>,
    /// The commits and authors counted, for the page head's tagline.
    said: String,
    kpis: Vec<Kpi>,
    langs: Vec<LangRow>,
    commits: Vec<views::CommitRow>,
    more: usize,
    diff_href: String,
    /// The head commit's short id, the link to it.
    /// The head commit as the shared sha chip.
    head_sha: String,
}

#[derive(Template)]
#[template(path = "compare.html")]
struct ComparePage {
    c: Chrome,
    page_head: PageHead,
    /// Set when there is no base yet, or the range cannot be shown: what to do.
    pick: Option<Empty>,
    error: Option<String>,
    result: Option<Result2>,
}

/// The base and head fields, the compare page head's toolbar. Enter submits.
#[derive(Template)]
#[template(path = "compare_form.html")]
struct CmpForm {
    action: String,
    base: String,
    head: String,
    merge: bool,
}

const COMPARE_LIST: usize = 200;

/// The most commits a comparison reads from git for a head the index lacks. Past it the page
/// asks for a nearer base instead of diffing an unbounded range inside one request.
/// Not configurable: a guard on request cost, not a preference.
const GIT_RANGE_CAP: usize = 2_000;

/// What a comparison found.
enum Range {
    Counted(Comparison),
    /// More than [`GIT_RANGE_CAP`] commits outside the index.
    TooLarge,
    /// The head is outside the index and the server is public, which reads only the index.
    Unindexed,
    /// A read failed: shown as such, never as an empty range.
    Failed(String),
}

/// The link to compare `base` with `head`, or `""` when the compare page is turned off. With
/// no `base` the page starts from the latest tag.
pub(crate) fn compare_href(base: Option<&str>, head: &str) -> String {
    crate::features::href_if(gitterrace_config::Feature::Compare, &compare_path(base, head, false))
}

/// The compare page's path and query, with no feature switch and no repo prefix. `merge_base`
/// asks for `base...head`, the commits `head` has since it left `base`.
pub fn compare_path(base: Option<&str>, head: &str, merge_base: bool) -> String {
    let base = base.map_or_else(String::new, |b| format!("base={}&", enc_seg(b)));
    let mode = if merge_base { format!("&mode={MERGE_BASE}") } else { String::new() };
    format!("/compare?{base}head={}{mode}", enc_seg(head))
}

/// The newest tag, by its commit's time: where a comparison starts when none is asked for.
pub(crate) fn latest_tag(refs: &[gitterrace_git::RefInfo]) -> Option<String> {
    refs.iter()
        .filter(|r| r.kind == gitterrace_git::RefKind::Tag)
        .max_by_key(|r| r.time)
        .map(|r| r.name.clone())
}

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<CompareQuery>,
) -> Result<Html<String>> {
    let (base, head) = (q.base.unwrap_or_default(), q.head.unwrap_or_else(|| "HEAD".to_owned()));
    let h2 = head.clone();
    let merge = q.mode.as_deref() == Some(MERGE_BASE);
    cached(&app, "cm", raw, move |a, r, c| {
        let c = c.active("log");
        let refs = a.repo.refs().unwrap_or_else(|e| {
            tracing::warn!("compare: refs unreadable, no default base: {e}");
            Vec::new()
        });
        let b2 = if base.is_empty() { latest_tag(&refs).unwrap_or_default() } else { base };
        let tags: Vec<(String, String)> = refs
            .iter()
            .filter(|x| x.kind == gitterrace_git::RefKind::Tag)
            .map(|x| (x.name.clone(), compare_href(Some(&x.name), "HEAD")))
            .take(12)
            .collect();
        let branches: Vec<(String, String)> = refs
            .iter()
            .filter(|x| x.kind == gitterrace_git::RefKind::Branch && x.name != h2)
            .take(12)
            .map(|x| (x.name.clone(), compare_href(Some(&x.name), &h2)))
            .collect();
        let pick = b2.is_empty().then(|| {
            Empty::new(format!("Choose a base to compare {h2} with: a branch, tag or commit."))
        });
        let mut error = None;
        let mut state = None;
        let mut result = None;
        if !b2.is_empty() {
            match (a.repo.resolve(&b2), a.repo.resolve(&h2)) {
                (Ok(bi), Ok(hi)) => match start_of(a, &bi, &hi, merge) {
                    Ok(from) => match outcome(a, r, (&b2, &h2), (&from, &hi)) {
                        Ok(o) => result = Some(o),
                        Err(e) => state = Some(e),
                    },
                    Err(e) => state = Some(e),
                },
                _ => error = Some("One of the revisions does not exist.".to_owned()),
            }
        }
        let said = match &result {
            Some(r) if r.empty.is_none() && merge => {
                format!("{} on {h2} since it left {b2}", r.said)
            }
            Some(r) if r.empty.is_none() => format!("{} from {b2} to {h2}", r.said),
            _ if b2.is_empty() => "Pick a base to see what changed since".to_owned(),
            _ => format!("From {b2} to {h2}"),
        };
        // The quick picks follow as links: the tags, then the other branches. A no-break space
        // keeps the first name with the word before it on a narrow screen.
        let mut tagline = Tagline::of(&said);
        for (i, (name, to)) in tags.iter().chain(&branches).enumerate() {
            let sep = match i {
                0 if !tags.is_empty() => " \u{b7} since\u{a0}",
                0 => " \u{b7} against\u{a0}",
                i if i == tags.len() && !tags.is_empty() => ", against\u{a0}",
                _ => ", ",
            };
            tagline = tagline.link(sep, name.clone(), to.clone());
        }
        let page_head = PageHead {
            qualifier: "commits".to_owned(),
            actions: vec![Action {
                label: "Commits",
                href: href(&format!("/log/{}", enc_seg(&h2))),
            }],
            toolbar: CmpForm { action: href("/compare"), base: b2, head: h2, merge }
                .render()
                .unwrap_or_default(),
            ..PageHead::new("Compare", tagline)
        };
        ComparePage { c, page_head, pick: pick.or(state), error, result }
    })
    .await
}

/// Where the range starts: `base` itself, or with `merge` the commit `head` left it from.
fn start_of(a: &App, base: &str, head: &str, merge: bool) -> std::result::Result<String, Empty> {
    if !merge {
        return Ok(base.to_owned());
    }
    match a.repo.merge_base(base, head) {
        Ok(Some(id)) => Ok(id),
        Ok(None) => Err(Empty::new(
            "These revisions share no history, so there is no point where one left the other.",
        )),
        Err(e) => {
            tracing::warn!("compare merge base: {e}");
            Err(Empty::new(
                "The repository could not be read for this comparison. Try again shortly.",
            ))
        }
    }
}

/// The figures, languages and commits of `base..head`, given by name and by resolved id, or
/// the state that stands in for them.
fn outcome(
    a: &App,
    r: &Src,
    (b2, h2): (&str, &str),
    (bi, hi): (&str, &str),
) -> std::result::Result<Result2, Empty> {
    let cmp = match comparison(a, r, bi, hi) {
        Range::Counted(c) => c,
        Range::TooLarge => {
            return Err(Empty::new(format!(
                "{h2} has more than {} that {b2} lacks, too many to read here. \
                 Compare with a nearer base.",
                fmt::count(GIT_RANGE_CAP, "commit", "commits")
            )));
        }
        Range::Unindexed => {
            return Err(Empty::new(format!(
                "{h2} is outside the history this server shows, so it cannot be compared here."
            )));
        }
        Range::Failed(e) => {
            tracing::warn!("compare {b2}..{h2}: {e}");
            return Err(Empty::new(
                "The repository could not be read for this comparison. Try again shortly.",
            ));
        }
    };
    let net = cmp.added.abs_diff(cmp.removed);
    let kpis = vec![
        Kpi("Commits without merges", fmt::thousands(cmp.commits.len())),
        Kpi("Authors", fmt::thousands(cmp.authors)),
        Kpi("Files changed", fmt::thousands(cmp.files_changed)),
        Kpi("Lines added", fmt::thousands(cmp.added)),
        Kpi("Lines removed", fmt::thousands(cmp.removed)),
        Kpi(
            "Net",
            format!("{}{}", if cmp.added >= cmp.removed { "+" } else { "-" }, fmt::thousands(net)),
        ),
    ];
    let langs = cmp
        .languages
        .iter()
        .map(|l| LangRow {
            language: l.language.clone(),
            added: fmt::thousands(l.added),
            removed: fmt::thousands(l.removed),
            net: format!(
                "{}{}",
                if l.net >= 0 { "+" } else { "-" },
                fmt::thousands(l.net.unsigned_abs())
            ),
            tone: if l.net >= 0 { "add" } else { "del" },
        })
        .collect();
    let shown: Vec<String> = cmp.commits.iter().take(COMPARE_LIST).cloned().collect();
    let now = fmt::now();
    let commits = match a.repo.summaries(&shown) {
        Ok(s) => s.iter().map(|s| views::CommitRow::of(s, now)).collect(),
        Err(e) => {
            tracing::warn!("compare {b2}..{h2} summaries: {e}");
            return Err(Empty::new(
                "The repository could not be read for this comparison. Try again shortly.",
            ));
        }
    };
    Ok(Result2 {
        said: format!(
            "{} by {}",
            fmt::count(cmp.commits.len(), "commit", "commits"),
            fmt::count(cmp.authors, "author", "authors")
        ),
        empty: cmp.commits.is_empty().then(|| {
            let e = Empty::new(format!("{h2} has no commits that {b2} lacks."));
            if bi == hi {
                return e;
            }
            e.step(format!("Compare the other way, {h2} to {b2}"), compare_href(Some(h2), b2))
        }),
        kpis,
        langs,
        commits,
        more: cmp.commits.len().saturating_sub(COMPARE_LIST),
        diff_href: href(&format!("/commit/{hi}")),
        head_sha: fmt::sha_link(hi),
    })
}

/// `base..head` from the index when it holds the head (then it holds the whole range), else
/// read from git: the index holds what the served head reaches, so another branch's own
/// commits are only in git. The git read is bounded and left to local servers.
fn comparison(a: &App, r: &Src, base: &str, head: &str) -> Range {
    lookup(a, r, base, head).unwrap_or_else(Range::Failed)
}

/// [`comparison`] with each read's failure propagated.
fn lookup(a: &App, r: &Src, base: &str, head: &str) -> std::result::Result<Range, String> {
    if r.find_commit(head).map_err(|e| e.to_string())?.is_some() {
        let ins = insights(a, r, a.defaults().into_options());
        return Ok(Range::Counted(ins.compare(base, head)));
    }
    if a.public() {
        return Ok(Range::Unindexed);
    }
    let key = format!("{base}..{head}");
    if let Some(c) = a.caches.compares.lock().ok().and_then(|g| g.get(&key)) {
        return Ok(Range::Counted((*c).clone()));
    }
    let range = from_git(a, base, head).map_err(|e| e.to_string())?;
    if let Range::Counted(c) = &range
        && let Ok(mut g) = a.caches.compares.lock()
    {
        g.put(key, Arc::new(c.clone()));
    }
    Ok(range)
}

/// [`comparison`] for a head the index lacks: at most [`GIT_RANGE_CAP`] commits, each
/// diffed once, matched to its summary by id.
fn from_git(a: &App, base: &str, head: &str) -> gitterrace_git::Result<Range> {
    a.git_ranges.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ids = a.repo.range(base, head, GIT_RANGE_CAP + 1)?;
    if ids.len() > GIT_RANGE_CAP {
        return Ok(Range::TooLarge);
    }
    let summaries = a.repo.summaries(&ids)?;
    let stats = a.repo.numstat_many(&ids)?;
    if summaries.len() != ids.len() || stats.len() != ids.len() {
        return Err(gitterrace_git::Error::RevNotFound(head.to_owned()));
    }
    let mut raw = Vec::with_capacity(ids.len());
    for (s, files) in summaries.into_iter().zip(stats) {
        let Some(files) = files else {
            return Err(gitterrace_git::Error::RevNotFound(s.id));
        };
        raw.push(RawCommit {
            author: s.author.email,
            merge: s.parents.len() > 1,
            files: files
                .into_iter()
                .map(|f| {
                    let (add, del) = (u64::from(f.additions), u64::from(f.deletions));
                    (f.path, add, del, u64::from(f.rewritten))
                })
                .collect(),
            id: s.id,
        });
    }
    Ok(Range::Counted(Comparison::of_raw(&raw)))
}

#[cfg(test)]
mod tests {
    use gitterrace_git::{RefInfo, RefKind};

    use super::*;

    fn r(name: &str, kind: RefKind, time: i64) -> RefInfo {
        RefInfo { name: name.to_owned(), kind, target: String::new(), time, annotation: None }
    }

    #[test]
    fn a_repeated_range_reads_git_once_and_a_failure_is_not_kept() {
        use std::sync::atomic::Ordering::Relaxed;
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path();
        let run = |args: &[&str]| gitterrace_testkit::git(root, args);
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "1\n").expect("write");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "feat: one"]);
        run(&["checkout", "-q", "-b", "side"]);
        std::fs::write(root.join("b.txt"), "x\ny\n").expect("write");
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "feat: two"]);
        let repo = gitterrace_git::Repo::discover(root).expect("repo");
        let index = gitterrace_index::Index::open(&repo).expect("index");
        let (base, head) =
            (repo.resolve("main").expect("main"), repo.resolve("side").expect("side"));
        let app = App::new(repo, index);
        let src: Src = Arc::new(gitterrace_index::feed::MemorySource::new(&[]));
        let first = lookup(&app, &src, &base, &head).expect("first");
        let second = lookup(&app, &src, &base, &head).expect("second");
        assert_eq!(app.git_ranges.load(Relaxed), 1, "the repeat reads nothing");
        match (first, second) {
            (Range::Counted(one), Range::Counted(two)) => assert_eq!(one, two),
            _ => panic!("both count"),
        }
        let missing = "0".repeat(40);
        assert!(lookup(&app, &src, &base, &missing).is_err());
        assert!(lookup(&app, &src, &base, &missing).is_err());
        assert_eq!(app.git_ranges.load(Relaxed), 3, "a failed read is retried");
    }

    #[test]
    fn the_latest_tag_is_the_newest_tag_not_the_newest_ref() {
        let refs =
            [r("main", RefKind::Branch, 30), r("v1", RefKind::Tag, 10), r("v2", RefKind::Tag, 20)];
        assert_eq!(latest_tag(&refs).as_deref(), Some("v2"));
        assert_eq!(latest_tag(&refs[..1]), None);
    }

    #[test]
    fn a_link_without_a_base_leaves_the_prefill_to_the_page() {
        let on = crate::features::Effective::of(&gitterrace_config::Config::default(), None);
        let (with, without) = crate::features::with(on, || {
            (compare_href(Some("v 1"), "main"), compare_href(None, "main"))
        });
        assert!(with.ends_with("/compare?base=v%201&head=main"), "{with}");
        assert!(without.ends_with("/compare?head=main"), "{without}");
    }
}
