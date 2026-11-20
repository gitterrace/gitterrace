//! Every period link an insights page renders lands on that period, selected and scoped.

use std::sync::Arc;

use gitterrace_testkit::Fixture;

use crate::{app::App, tests::get};

const PAGES: [&str; 5] = [
    "/stats",
    "/stats/hotspots",
    "/stats/ownership",
    "/stats/contributors",
    "/stats/languages",
];

/// A repository with one commit years ago and one today, so Week and All count differently.
pub(crate) fn app(dir: &std::path::Path) -> Arc<App> {
    let f = Fixture::init(dir)
        .at("2020-01-06T12:00:00Z")
        .write("old.rs", "a\nb\n")
        .commit("feat(core): old");
    // Today's commit goes around the fixture, whose pinned date holds for every call.
    std::fs::write(f.path().join("new.rs"), "c\n").expect("write");
    gitterrace_testkit::git(f.path(), &["add", "-A"]);
    gitterrace_testkit::git(f.path(), &["commit", "-q", "-m", "fix(core): new"]);
    crate::tests::ready_app(f.path())
}

/// The `(label, href, on)` of each choice in the page's Period control.
fn period_choices(body: &str) -> Vec<(String, String, bool)> {
    let Some(seg) = body.split("aria-label=\"Period\" data-seg-select>").nth(1) else {
        return Vec::new();
    };
    let seg = seg
        .split("</span>")
        .find(|s| s.contains("<a "))
        .unwrap_or_default();
    seg.split("<a href=\"")
        .skip(1)
        .map(|a| {
            let href = a
                .split('"')
                .next()
                .unwrap_or_default()
                .replace("&amp;", "&");
            let on = a
                .split('>')
                .next()
                .unwrap_or_default()
                .contains("aria-current");
            let label = a
                .split_once('>')
                .and_then(|(_, r)| r.split("</a>").next())
                .unwrap_or_default()
                .to_owned();
            (label, href, on)
        })
        .collect()
}

fn selected(body: &str) -> Option<String> {
    period_choices(body).into_iter().find(|c| c.2).map(|c| c.0)
}

/// The page body without its chrome and period control, for "the numbers moved" checks.
fn main_part(body: &str) -> String {
    let main = body.split("<main").nth(1).unwrap_or(body);
    main.split("data-seg-select>")
        .map(|s| s.split("</div>").skip(1).collect::<Vec<_>>().join("</div>"))
        .collect()
}

#[tokio::test]
async fn every_period_link_lands_selected_and_scoped() {
    let dir = tempfile::tempdir().expect("tmp");
    let app = app(dir.path());
    for page in PAGES {
        // The unfiltered page first, as the warm pass serves it, so a memo that ignores the
        // period answers every later link with it.
        let (s, body) = get(&app, page).await;
        assert_eq!(s, 200, "{page}");
        let choices = period_choices(&body);
        assert!(choices.len() >= 2, "{page} shows a period picker");
        let mut seen: Vec<(String, String)> = Vec::new();
        for (label, href, _) in choices {
            let (s, landed) = get(&app, &href).await;
            assert_eq!(s, 200, "{href}");
            assert_eq!(selected(&landed).as_deref(), Some(label.as_str()), "{href}");
            seen.push((label, main_part(&landed)));
        }
        let of = |l: &str| seen.iter().find(|(k, _)| k == l).map(|(_, b)| b.clone());
        assert_ne!(of("Week"), of("All"), "{page}: Week and All count the same");
    }
}

#[tokio::test]
async fn old_period_parameter_links_still_select_the_period() {
    let dir = tempfile::tempdir().expect("tmp");
    let app = app(dir.path());
    for page in PAGES {
        let (_, body) = get(&app, &format!("{page}?period=1w")).await;
        assert_eq!(selected(&body).as_deref(), Some("Week"), "{page}?period=1w");
    }
}
