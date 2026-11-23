//! The knowledge risk section of the ownership page and the page of one person.

use gitterrace_testkit::{Fixture, page};

/// Ann Lee wrote `old/` years ago and stopped. Bob wrote `new/` and is still committing.
fn history(dir: &std::path::Path) {
    let mut f = Fixture::init(dir).at("2020-01-10T12:00:00Z");
    f = f.write("old/a.txt", &"a\n".repeat(60)).write("old/b.txt", &"b\n".repeat(30));
    f = f.commit_by("Ann Lee <ann@x.io>", "feat: old code");
    f = f.at("2024-06-01T12:00:00Z").write("new/c.txt", &"c\n".repeat(40));
    let _ = f.commit_by("Bob <bob@x.io>", "feat: new code");
}

#[tokio::test]
async fn a_directory_of_an_idle_owner_is_at_risk_and_links_to_the_owner() {
    let dir = tempfile::tempdir().expect("tmp");
    history(dir.path());
    let router = crate::common::ready_router(dir.path()).await;
    let (status, body) = page(&router, "/stats/ownership").await;
    assert_eq!(status, 200);
    let risk = body.split("id=\"risk\"").nth(1).expect("the risk section");
    let old = risk.split("<tr data-risk-row>").find(|r| r.contains(">old<")).expect("old row");
    assert!(old.contains("At risk"), "{old}");
    assert!(old.contains("/stats/ownership/Ann%20Lee"), "no link to the owner: {old}");
    let new = risk.split("<tr data-risk-row>").find(|r| r.contains(">new<")).expect("new row");
    assert!(!new.contains("At risk"), "{new}");
    assert!(body.contains("id=\"orphans\""), "no files without an active author");
}

#[tokio::test]
async fn a_person_page_lists_what_they_own_and_an_unknown_name_is_not_found() {
    let dir = tempfile::tempdir().expect("tmp");
    history(dir.path());
    let router = crate::common::ready_router(dir.path()).await;
    let (status, body) = page(&router, "/stats/ownership/Ann%20Lee").await;
    assert_eq!(status, 200);
    assert!(body.contains("Ann Lee"), "{body}");
    assert!(body.contains("old/a.txt") || body.contains(">a.txt<"), "owned file not listed");
    assert!(!body.contains("new/c.txt"), "a file of another person is listed");
    let (status, _) = page(&router, "/stats/ownership/Nobody").await;
    assert_eq!(status, 404);
    let (status, body) = page(&router, "/stats/ownership/%3Cb%3Ezz").await;
    assert_eq!(status, 404);
    assert!(!body.contains("<b>zz"), "an unknown name is echoed unescaped");
}

#[tokio::test]
async fn drilling_lists_one_directory_and_leads_back_up() {
    let dir = tempfile::tempdir().expect("tmp");
    history(dir.path());
    let router = crate::common::ready_router(dir.path()).await;
    let (status, body) = page(&router, "/stats/ownership?drill=old").await;
    assert_eq!(status, 200);
    let risk = body.split("id=\"risk\"").nth(1).expect("the risk section");
    assert!(risk.contains("data-risk-up"), "no way back up");
    assert!(!risk.contains(">new<"), "a sibling directory is listed");
}
