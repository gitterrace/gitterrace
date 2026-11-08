//! A 404 inside one repository keeps the app shell: the header, search and nav stay, and the
//! page names what was missing and offers the next steps (search for it, its history, the
//! tree). A dead link from a note is a common way in, so the page must lead somewhere.

use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::{Html, IntoResponse, Response},
};
use gitterrace_config::Feature;
use gitterrace_git::percent_decode;

use crate::{
    app::{App, blocking},
    fmt::{enc_path, enc_seg, href},
    views::{self, Chrome},
};

/// Route prefixes whose answers scripts read: never wrapped in the page shell.
const FRAGMENTS: [&str; 5] = ["/api/", "/diff/", "/hl/", "/worktree/diff/", "/events"];

/// What a 404 said, carried on the response so the repository layer can re-render it.
#[derive(Clone)]
pub(crate) struct Missing(pub(crate) String);

#[derive(Template)]
#[template(path = "lost.html")]
struct LostPage {
    c: Chrome,
    head: crate::pagehead::PageHead,
    steps: Vec<Step>,
}

/// One next step on the page: a label and where it goes.
struct Step {
    label: String,
    href: String,
}

/// Route layer of a repository: a 404 its handlers answered is shown in the shell.
pub(crate) async fn in_shell(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_owned();
    // Fragments and JSON are read by scripts, which want the bare answer.
    if FRAGMENTS.iter().any(|p| path.starts_with(p)) {
        return next.run(req).await;
    }
    let rev = crate::fmt::rev_param(req.uri().query());
    let res = next.run(req).await;
    if res.status() != StatusCode::NOT_FOUND {
        return res;
    }
    let Some(Missing(detail)) = res.extensions().get::<Missing>().cloned() else {
        return res;
    };
    let shown = blocking(&app, move |a| {
        let (title, steps) = explain(&path, &rev);
        Ok(LostPage {
            c: views::chrome(a, ""),
            head: crate::pagehead::PageHead {
                qualifier: "404".to_owned(),
                ..crate::pagehead::PageHead::new(title, crate::pagehead::Tagline::of(&detail))
            },
            steps,
        }
        .render()?)
    })
    .await;
    // The bare page is still a 404 with the detail; the shell is a nicety.
    shown.map_or(res, |html| {
        (StatusCode::NOT_FOUND, Html(html)).into_response()
    })
}

/// The heading and the next steps for a missing `path` (the URL path inside the repository).
fn explain(path: &str, rev: &str) -> (String, Vec<Step>) {
    let mut parts = path.trim_start_matches('/').splitn(3, '/');
    let kind = parts.next().unwrap_or_default();
    let at = parts.next().map(percent_decode);
    let file = parts.next().map(percent_decode).filter(|f| !f.is_empty());
    let mut steps = Vec::new();
    let rev = at
        .clone()
        .filter(|_| matches!(kind, "blob" | "tree" | "blame" | "raw" | "log"))
        .unwrap_or_else(|| rev.to_owned());
    let title = match (kind, &file) {
        ("blob" | "blame" | "raw", Some(f)) => format!("No file at {f} on {rev}"),
        ("tree", Some(f)) => format!("No folder at {f} on {rev}"),
        ("blob" | "tree" | "blame" | "raw" | "log" | "commit", _) => {
            format!("No revision {}", at.as_deref().unwrap_or(&rev))
        }
        _ => "Nothing at this address".to_owned(),
    };
    if let Some(f) = &file {
        let name = f.rsplit('/').next().unwrap_or(f);
        if crate::features::on(Feature::Search) {
            steps.push(Step {
                label: format!("Search for {name}"),
                href: href(&format!("/search?q={}", enc_seg(name))),
            });
        }
        if crate::features::on(Feature::History) {
            steps.push(Step {
                label: "Open the history of this path".to_owned(),
                href: href(&format!("/history/{}", enc_path(f))),
            });
        }
    }
    let known = !title.starts_with("No revision");
    steps.push(if known && rev != "HEAD" {
        Step {
            label: format!("Back to the tree of {rev}"),
            href: href(&format!("/tree/{}", enc_seg(&rev))),
        }
    } else {
        Step {
            label: "Back to the overview".to_owned(),
            href: href("/"),
        }
    });
    (title, steps)
}

#[cfg(test)]
mod tests {
    use super::explain;

    #[test]
    fn a_missing_file_offers_search_history_and_the_tree() {
        let (title, steps) = explain("/blob/v1/src/nope.c", "HEAD");
        assert_eq!(title, "No file at src/nope.c on v1");
        let labels: Vec<_> = steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Search for nope.c",
                "Open the history of this path",
                "Back to the tree of v1"
            ]
        );
        assert!(steps[1].href.ends_with("/history/src/nope.c"));
    }

    #[test]
    fn a_bad_revision_leads_back_to_the_overview() {
        let (title, steps) = explain("/log/nope", "HEAD");
        assert_eq!(title, "No revision nope");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].label, "Back to the overview");
    }
}
