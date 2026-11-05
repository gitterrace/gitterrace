//! The compare page, `/compare`: what changed between two revisions. It belongs to Commits: the
//! top nav marks Commits, the log links here, and an empty form starts from the latest tag.
//! Every link to the page comes from [`compare_href`], so the prefill and the feature switch
//! hold on all of them.

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
    routes::Shared,
    views::{self, Chrome},
};

#[derive(Deserialize, Default)]
pub(crate) struct CompareQuery {
    base: Option<String>,
    head: Option<String>,
}

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
    kpis: Vec<Kpi>,
    langs: Vec<LangRow>,
    commits: Vec<views::CommitRow>,
    more: usize,
    diff_href: String,
}

#[derive(Template)]
#[template(path = "compare.html")]
struct ComparePage {
    c: Chrome,
    /// The commits of the head, the page this one sits under.
    log_href: String,
    base: String,
    head: String,
    tags: Vec<(String, String)>,
    /// Other branches, each a link comparing it with the head.
    branches: Vec<(String, String)>,
    /// Set when there is no base yet: what to do first.
    pick: Option<Empty>,
    error: Option<String>,
    result: Option<Result2>,
}

const COMPARE_LIST: usize = 200;

/// The link to compare `base` with `head`, or `""` when the compare page is turned off. With
/// no `base` the page starts from the latest tag.
pub(crate) fn compare_href(base: Option<&str>, head: &str) -> String {
    let base = base.map_or_else(String::new, |b| format!("base={}&", enc_seg(b)));
    let path = format!("/compare?{base}head={}", enc_seg(head));
    crate::features::href_if(gitterrace_config::Feature::Compare, &path)
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
    let (base, head) = (
        q.base.unwrap_or_default(),
        q.head.unwrap_or_else(|| "HEAD".to_owned()),
    );
    let h2 = head.clone();
    cached(&app, "cm", raw, move |a, r, c| {
        let c = c.active("log");
        let refs = a.repo.refs().unwrap_or_default();
        let b2 = if base.is_empty() {
            latest_tag(&refs).unwrap_or_default()
        } else {
            base
        };
        let tags = refs
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
            Empty::new(format!(
                "Choose a base to compare {h2} with: a branch, tag or commit."
            ))
        });
        let mut error = None;
        let mut result = None;
        if !b2.is_empty() {
            match (a.repo.resolve(&b2), a.repo.resolve(&h2)) {
                (Ok(bi), Ok(hi)) => result = Some(outcome(a, r, (&b2, &h2), (&bi, &hi))),
                _ => error = Some("One of the revisions does not exist.".to_owned()),
            }
        }
        ComparePage {
            log_href: href(&format!("/log/{}", enc_seg(&h2))),
            c,
            base: b2,
            head: h2,
            tags,
            branches,
            pick,
            error,
            result,
        }
    })
    .await
}

/// The figures, languages and commits of `base..head`, given by name and by resolved id.
fn outcome(a: &App, r: &Src, (b2, h2): (&str, &str), (bi, hi): (&str, &str)) -> Result2 {
    let cmp = comparison(a, r, bi, hi);
    let net = cmp.added.abs_diff(cmp.removed);
    let kpis = vec![
        Kpi("Commits without merges", fmt::thousands(cmp.commits.len())),
        Kpi("Authors", fmt::thousands(cmp.authors)),
        Kpi("Files changed", fmt::thousands(cmp.files_changed)),
        Kpi("Lines added", fmt::thousands(cmp.added)),
        Kpi("Lines removed", fmt::thousands(cmp.removed)),
        Kpi(
            "Net",
            format!(
                "{}{}",
                if cmp.added >= cmp.removed { "+" } else { "-" },
                fmt::thousands(net)
            ),
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
    let commits = a
        .repo
        .summaries(&shown)
        .unwrap_or_default()
        .iter()
        .map(|s| views::CommitRow::of(s, now))
        .collect();
    Result2 {
        empty: cmp.commits.is_empty().then(|| {
            let e = Empty::new(format!("{h2} has no commits that {b2} lacks."));
            if bi == hi {
                return e;
            }
            e.step(
                format!("Compare the other way, {h2} to {b2}"),
                compare_href(Some(h2), b2),
            )
        }),
        kpis,
        langs,
        commits,
        more: cmp.commits.len().saturating_sub(COMPARE_LIST),
        diff_href: href(&format!("/commit/{hi}")),
    }
}

/// `base..head` from the index, or read from git when the index lacks commits of the range:
/// the index holds what the served head reaches, so another branch's own commits are only in git.
fn comparison(a: &App, r: &Src, base: &str, head: &str) -> Comparison {
    let ins = insights(a, r, a.defaults().into_options());
    let cmp = ins.compare(base, head);
    let ids = a.repo.range(base, head).unwrap_or_default();
    if ids.len() <= cmp.commits.len() {
        return cmp;
    }
    let summaries = a.repo.summaries(&ids).unwrap_or_default();
    let stats = a.repo.numstat_many(&ids).unwrap_or_default();
    let raw: Vec<RawCommit> = summaries
        .iter()
        .zip(stats)
        .map(|(s, files)| RawCommit {
            id: s.id.clone(),
            author: s.author.email.clone(),
            merge: s.parents.len() > 1,
            files: files
                .unwrap_or_default()
                .into_iter()
                .map(|f| {
                    let (add, del) = (u64::from(f.additions), u64::from(f.deletions));
                    (f.path, add, del, u64::from(f.rewritten))
                })
                .collect(),
        })
        .collect();
    Comparison::of_raw(&raw)
}

#[cfg(test)]
mod tests {
    use gitterrace_git::{RefInfo, RefKind};

    use super::*;

    fn r(name: &str, kind: RefKind, time: i64) -> RefInfo {
        RefInfo {
            name: name.to_owned(),
            kind,
            target: String::new(),
            time,
            annotation: None,
        }
    }

    #[test]
    fn the_latest_tag_is_the_newest_tag_not_the_newest_ref() {
        let refs = [
            r("main", RefKind::Branch, 30),
            r("v1", RefKind::Tag, 10),
            r("v2", RefKind::Tag, 20),
        ];
        assert_eq!(latest_tag(&refs).as_deref(), Some("v2"));
        assert_eq!(latest_tag(&refs[..1]), None);
    }

    #[test]
    fn a_link_without_a_base_leaves_the_prefill_to_the_page() {
        let on = crate::features::Effective::of(&gitterrace_config::Config::default(), None);
        let (with, without) = crate::features::with(on, || {
            (
                compare_href(Some("v 1"), "main"),
                compare_href(None, "main"),
            )
        });
        assert!(with.ends_with("/compare?base=v%201&head=main"), "{with}");
        assert!(without.ends_with("/compare?head=main"), "{without}");
    }
}
