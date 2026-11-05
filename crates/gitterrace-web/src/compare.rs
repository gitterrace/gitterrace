//! The compare page, `/compare`: what changed between two revisions. It belongs to Commits: the
//! top nav marks Commits, the log links here, and an empty form starts from the latest tag.
//! Every link to the page comes from [`compare_href`], so the prefill and the feature switch
//! hold on all of them.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use serde::Deserialize;

use crate::{
    error::Result,
    fmt::{self, enc_seg, href},
    insights::{Kpi, cached, insights},
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
        let mut error = None;
        let mut result = None;
        if !b2.is_empty() {
            match (a.repo.resolve(&b2), a.repo.resolve(&h2)) {
                (Ok(bi), Ok(hi)) => {
                    let ins = insights(a, r, a.defaults().into_options());
                    let cmp = ins.compare(&bi, &hi);
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
                    let shown: Vec<String> =
                        cmp.commits.iter().take(COMPARE_LIST).cloned().collect();
                    let now = fmt::now();
                    let commits = a
                        .repo
                        .summaries(&shown)
                        .unwrap_or_default()
                        .iter()
                        .map(|s| views::CommitRow::of(s, now))
                        .collect();
                    result = Some(Result2 {
                        kpis,
                        langs,
                        commits,
                        more: cmp.commits.len().saturating_sub(COMPARE_LIST),
                        diff_href: href(&format!("/commit/{hi}")),
                    });
                }
                _ => error = Some("One of the revisions does not exist.".to_owned()),
            }
        }
        ComparePage {
            log_href: href(&format!("/log/{}", enc_seg(&h2))),
            c,
            base: b2,
            head: h2,
            tags,
            error,
            result,
        }
    })
    .await
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
