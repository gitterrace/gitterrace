//! The scope switcher of the commit and compare pages: This commit | Branch vs default branch |
//! Working tree. One function builds every href, so a page never links to a route that is off,
//! not served on a public server, or has no base to compare with.

use gitterrace_config::Feature;

use crate::{
    compare::compare_path,
    features,
    fmt::enc_seg,
    seg::{Choice, Seg},
};

/// The page the switcher sits on.
pub(crate) enum Page<'a> {
    Commit { id: &'a str },
    Compare { base: &'a str, head: &'a str },
}

/// What the server shows: the default branch to compare with (none when HEAD is detached) and
/// whether this machine's working tree is served (never on a public server).
pub(crate) struct Scope {
    pub(crate) default_branch: Option<String>,
    pub(crate) worktree: bool,
}

impl Scope {
    /// What `app` serves: the working tree is this machine's state, so never on a public server.
    pub(crate) fn of(app: &crate::app::App) -> Self {
        Self { default_branch: app.repo.head_branch(), worktree: !app.public() }
    }
}

/// The entries that apply to `page`, in order. Fewer than two is no choice, so it is empty.
pub(crate) fn choices(page: &Page, scope: &Scope) -> Vec<Choice> {
    let (id, current_base) = match *page {
        Page::Commit { id } => (id, None),
        Page::Compare { base, head } => (head, Some(base)),
    };
    let mut out = vec![Choice::new(
        "This commit",
        crate::fmt::href(&format!("/commit/{}", enc_seg(id))),
        current_base.is_none(),
    )];
    if let Some(default) = scope.default_branch.as_deref() {
        let to = features::href_if(Feature::Compare, &compare_path(Some(default), id, true));
        if !to.is_empty() {
            out.push(Choice::new("Branch vs default branch", to, current_base == Some(default)));
        }
    }
    if scope.worktree {
        let to = features::href_if(Feature::Worktree, "/worktree");
        if !to.is_empty() {
            out.push(Choice::new("Working tree", to, false));
        }
    }
    if out.len() < 2 {
        out.clear();
    }
    out
}

/// The switcher's markup, or `""` when there is nothing to switch between.
pub(crate) fn render(page: &Page, scope: &Scope) -> String {
    let c = choices(page, scope);
    if c.is_empty() { String::new() } else { Seg::links("Scope", c).no_caption().to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Scope {
        Scope { default_branch: Some("main".to_owned()), worktree: true }
    }

    fn with_features<T>(toml: &str, f: impl FnOnce() -> T) -> T {
        let cfg = gitterrace_config::parse(toml).expect("parse").config;
        features::with(features::Effective::of(&cfg, None), f)
    }

    fn targets(c: &[Choice]) -> Vec<(&str, &str, bool)> {
        c.iter().map(|c| (c.label.as_str(), c.target.as_str(), c.on)).collect()
    }

    #[test]
    fn a_commit_page_offers_its_commit_the_branch_and_the_working_tree() {
        let c = with_features("", || choices(&Page::Commit { id: "abc" }, &all()));
        assert_eq!(
            targets(&c),
            [
                ("This commit", "/commit/abc", true),
                ("Branch vs default branch", "/compare?base=main&head=abc&mode=merge-base", false),
                ("Working tree", "/worktree", false),
            ]
        );
    }

    #[test]
    fn a_compare_page_marks_the_branch_entry_only_against_the_default_branch() {
        let page = Page::Compare { base: "main", head: "feat" };
        let on = with_features("", || choices(&page, &all()));
        assert_eq!(targets(&on)[0], ("This commit", "/commit/feat", false));
        assert!(on[1].on && !on[0].on && !on[2].on);
        let page = Page::Compare { base: "v1", head: "feat" };
        let off = with_features("", || choices(&page, &all()));
        assert!(off.iter().all(|c| !c.on));
    }

    #[test]
    fn entries_that_do_not_apply_are_left_out() {
        let page = Page::Commit { id: "abc" };
        let none = Scope { default_branch: None, worktree: true };
        let c = with_features("", || choices(&page, &none));
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["This commit", "Working tree"]);
        let public = Scope { default_branch: Some("main".to_owned()), worktree: false };
        let c = with_features("", || choices(&page, &public));
        assert!(c.iter().all(|c| c.label != "Working tree"));
        let c = with_features("[features]\nworktree_enabled = false\n", || choices(&page, &all()));
        assert!(c.iter().all(|c| c.label != "Working tree"), "{:?}", targets(&c));
        let c = with_features("[features]\ncompare_enabled = false\n", || choices(&page, &all()));
        assert!(c.iter().all(|c| !c.target.contains("/compare")), "{:?}", targets(&c));
    }

    #[test]
    fn a_lone_entry_is_no_switcher() {
        let alone = Scope { default_branch: None, worktree: false };
        assert!(with_features("", || choices(&Page::Commit { id: "abc" }, &alone)).is_empty());
        assert_eq!(with_features("", || render(&Page::Commit { id: "abc" }, &alone)), "");
        let html = with_features("", || render(&Page::Commit { id: "abc" }, &all()));
        assert!(html.contains("aria-label=\"Scope\"") && html.contains("aria-current"), "{html}");
    }

    #[test]
    fn a_revision_with_odd_characters_is_encoded_in_every_href() {
        let c = with_features("", || choices(&Page::Commit { id: "a/b c" }, &all()));
        assert!(c.iter().all(|c| !c.target.contains(' ')), "{:?}", targets(&c));
    }

    /// Served under `/r/m/`, every switcher link keeps the repository prefix and the commit page
    /// and the compare page both carry it. Catches a href built without `fmt::href`.
    #[tokio::test]
    async fn the_switcher_links_keep_the_repository_base() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = gitterrace_testkit::Fixture::init(&dir.path().join("m"))
            .write("a.txt", "a\n")
            .commit("feat: one")
            .write("a.txt", "b\n")
            .commit("feat: two");
        let router = crate::app_router(f.path()).expect("router");
        // The compare page waits for the history index, built in the background.
        gitterrace_testkit::wait::page_until(
            &router,
            "/r/m/stats",
            gitterrace_testkit::wait::indexed,
        )
        .await;
        for uri in ["/r/m/commit/HEAD", "/r/m/compare?base=HEAD~1&head=HEAD"] {
            let (st, page) = gitterrace_testkit::get(&router, uri).await;
            assert_eq!(st, 200, "{uri}");
            let at = page.find("aria-label=\"Scope\"").expect("a switcher");
            let hrefs: Vec<&str> = page[at..]
                .split("<select")
                .next()
                .unwrap_or_default()
                .split("href=\"")
                .skip(1)
                .filter_map(|h| h.split('"').next())
                .collect();
            assert!(hrefs.len() >= 2, "{uri}: {hrefs:?}");
            assert!(hrefs.iter().all(|h| h.starts_with("/r/m/")), "{uri}: {hrefs:?}");
        }
    }
}
