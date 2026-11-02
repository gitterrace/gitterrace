//! The compare page, `/compare`: what changed between two revisions.

use askama::Template;
use axum::extract::{Query, RawQuery, State};
use axum::response::Html;
use serde::Deserialize;

use crate::{
    error::Result,
    fmt::{self, enc_seg, href},
    insights::{Kpi, cached, insights},
    routes::Shared,
    views::{self, Chrome, Tabs},
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

struct Listed {
    href: String,
    sha: String,
    subject: String,
}

struct Result2 {
    kpis: Vec<Kpi>,
    langs: Vec<LangRow>,
    commits: Vec<Listed>,
    more: usize,
    diff_href: String,
}

#[derive(Template)]
#[template(path = "compare.html")]
struct ComparePage {
    c: Chrome,
    tabs: Tabs,
    base: String,
    head: String,
    tags: Vec<(String, String)>,
    error: Option<String>,
    result: Option<Result2>,
}

const COMPARE_LIST: usize = 200;

pub(crate) async fn page(
    State(app): Shared,
    RawQuery(raw): RawQuery,
    Query(q): Query<CompareQuery>,
) -> Result<Html<String>> {
    let (base, head) = (
        q.base.unwrap_or_default(),
        q.head.unwrap_or_else(|| "HEAD".to_owned()),
    );
    let b2 = base.clone();
    let h2 = head.clone();
    cached(&app, "cm", raw, move |a, r, c| {
        let tags = a
            .repo
            .refs()
            .unwrap_or_default()
            .into_iter()
            .filter(|x| x.kind == gitterrace_git::RefKind::Tag)
            .map(|x| {
                (
                    x.name.clone(),
                    href(&format!("/compare?base={}&head=HEAD", enc_seg(&x.name))),
                )
            })
            .take(12)
            .collect();
        let mut error = None;
        let mut result = None;
        if !b2.is_empty() {
            match (a.repo.resolve(&b2), a.repo.resolve(&h2)) {
                (Ok(bi), Ok(hi)) => {
                    let ins = insights(a, r, a.defaults().into_options());
                    let cmp = ins.compare(&bi, &hi);
                    let subjects = a.subjects(&hi);
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
                    let commits = cmp
                        .commits
                        .iter()
                        .take(COMPARE_LIST)
                        .map(|id| Listed {
                            href: href(&format!("/commit/{id}")),
                            sha: fmt::sha_link(id),
                            subject: subjects.get(id).cloned().unwrap_or_default(),
                        })
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
            c,
            tabs: views::stats_tabs("cm"),
            base: b2,
            head: h2,
            tags,
            error,
            result,
        }
    })
    .await
}
