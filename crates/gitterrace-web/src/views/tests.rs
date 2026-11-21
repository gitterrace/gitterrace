use crate::views::{code_html, code_html_first, code_tail_html, crumbs};

/// The default limits with a 32 MiB code cache.
fn cache() -> crate::views::Render {
    crate::views::Render::test(32 << 20)
}

#[test]
fn crumbs_link_prefixes() {
    let c = crumbs("main", "a/b.rs", true);
    assert_eq!(c.len(), 3);
    assert_eq!(c[1].href, "/tree/main/a");
    assert_eq!(c[2].href, "/blob/main/a/b.rs");
}

#[test]
fn tail_of_a_file_without_late_chunks_is_empty() {
    let ten = "fn main() {}\n".repeat(10);
    let two_chunks = "fn main() {}\n".repeat(150);
    for (path, text) in [
        ("a.rs", ""),
        ("a.rs", ten.as_str()),
        ("a.rs", two_chunks.as_str()),
        ("a.unknownext", ten.as_str()),
    ] {
        assert_eq!(code_tail_html(&cache(), path, text), "", "{path}");
    }
}

#[test]
fn code_escapes() {
    let (h, cut) = code_html(&cache(), "x.txt", "<b>\nx");
    assert!(h.contains("&lt;b&gt;"));
    assert!(!cut);
    assert!(h.contains("id=\"L2\""));
    assert!(!h.contains("<table"));
}

#[test]
fn number_column_width_follows_the_digit_count() {
    let digits = |n: usize| {
        let (h, _) = code_html(&cache(), "a.txt", &"x\n".repeat(n));
        h.split("data-digits=\"").nth(1).unwrap().chars().next().unwrap()
    };
    assert_eq!((digits(5), digits(999), digits(1_000), digits(12_345)), ('3', '3', '4', '5'));
}

#[test]
fn big_files_light_the_first_chunks_and_the_tail_completes_them() {
    let text = "fn f() { let x = 1; }\n".repeat(3_000);
    let (first, _) = code_html_first(&cache(), "a.rs", &text);
    assert!(first.starts_with("<div class=\"code src\" data-hl data-digits=\"4\">"));
    let lit = |h: &str| h.matches("s-keyword").count();
    assert!(lit(&first) > 0 && lit(&first) < 3_000 * 2);
    let (full, _) = code_html(&cache(), "a.rs", &text);
    let tail = code_tail_html(&cache(), "a.rs", &text);
    let head_end = first.match_indices("<div class=\"ch\"").nth(2).unwrap().0;
    let glued = format!(
        "{}{}</div>",
        &full[..full.match_indices("<div class=\"ch\"").nth(2).unwrap().0],
        tail
    );
    assert_eq!(glued.replace(" data-hl", ""), full);
    assert!(first[..head_end].contains("s-keyword"));
    let (small, _) = code_html_first(&cache(), "a.rs", "fn f() {}\n");
    assert!(!small.contains("data-hl"));
}

#[tokio::test]
async fn diff_fragment_out_of_range_is_not_found() {
    use axum::extract::{Path, State};
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let _fixture = gitterrace_testkit::Fixture::init(p)
        .write(
            "a.txt", "x
",
        )
        .commit("feat: a");
    let repo = gitterrace_git::Repo::discover(p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    let app = std::sync::Arc::new(crate::app::App::new(repo, index));
    let ask = |i: usize| super::diff_fragment(State(app.clone()), Path(("HEAD".into(), i)));
    assert!(ask(0).await.is_ok());
    assert!(ask(99_999).await.is_err());
}

#[test]
fn code_lines_carry_their_number() {
    let text = "l\n".repeat(250);
    let (h, _) = code_html(&cache(), "x.txt", &text);
    assert_eq!(h.matches("class=\"ch\"").count(), 3);
    // A strict CSP refuses inline styles, so each line names its own number.
    assert!(h.contains("id=\"L101\" class=\"l\" data-n=\"101\""));
    assert!(h.contains("id=\"L250\""));
    assert!(!h.contains("style="));
}

#[test]
fn every_insights_tab_is_in_the_row_and_the_open_one_is_marked() {
    let t = crate::views::stats_tabs("ow");
    let on: Vec<_> = t.iter().filter(|n| n.on).map(|n| n.label).collect();
    assert_eq!(on, ["Ownership"]);
    assert!(t.len() <= 6, "{} tabs", t.len());
}

#[test]
fn a_commit_landed_later_joins_the_day_it_landed() {
    use gitterrace_git::CommitSummary;
    use gitterrace_types::api::Signature;
    const DAY: i64 = 86_400;
    let now = 20 * DAY + 12 * 3600;
    let sig = |time| Signature { email: "a@x".to_owned(), name: "A".to_owned(), offset: 0, time };
    // Newest first, as the log lists them: today, a rebase written yesterday but landed today,
    // then yesterday.
    let c = |id: &str, wrote: i64, landed: i64| CommitSummary {
        id: id.to_owned(),
        parents: Vec::new(),
        author: sig(wrote),
        committer: sig(landed),
        subject: id.to_owned(),
    };
    let list = [
        c("a", now - 3600, now - 3600),
        c("b", now - DAY, now - 7200),
        c("c", now - 2 * 3600, now - 2 * 3600),
        c("d", now - DAY - 3600, now - DAY - 3600),
    ];
    let labels: Vec<String> =
        crate::views::day_groups(&list, now).into_iter().map(|g| g.label).collect();
    assert_eq!(labels, ["Today", "Yesterday"]);
}

/// The log groups by the day a commit landed, every count by its author's local day. A commit
/// written Monday 23:30 in Berlin and committed on Tuesday sits under Tuesday and counts Monday.
#[test]
fn a_monday_night_commit_lands_under_tuesday_and_counts_on_monday() {
    use gitterrace_git::CommitSummary;
    use gitterrace_types::api::Signature;
    const DAY: i64 = 86_400;
    const BERLIN: i32 = 3600;
    // 2026-11-02 is a Monday.
    let monday = 20_759 * DAY;
    let sig =
        |time| Signature { email: "a@x".to_owned(), name: "A".to_owned(), offset: BERLIN, time };
    // Local 23:30 Monday is 22:30 UTC Monday; the commit landed Tuesday 10:00 local.
    let wrote = monday + 22 * 3600 + 1800;
    let landed = monday + DAY + 9 * 3600;
    let c = CommitSummary {
        id: "a".to_owned(),
        parents: Vec::new(),
        author: sig(wrote),
        committer: sig(landed),
        subject: "a".to_owned(),
    };
    let now = landed + 3600;
    let labels: Vec<String> = crate::views::day_groups(std::slice::from_ref(&c), now)
        .into_iter()
        .map(|g| g.label)
        .collect();
    assert_eq!(labels, ["Today"]);
    let counted = gitterrace_index::feed::local_day_of(c.author.time, c.author.offset);
    assert_eq!(counted, 20_759, "the bars count the author's Monday");
    assert_eq!(
        gitterrace_index::feed::local_day_of(c.committer.time, c.committer.offset),
        20_760,
        "the log heading is the committer's Tuesday"
    );
}

#[test]
fn no_day_heading_appears_twice_in_a_log() {
    use gitterrace_git::CommitSummary;
    use gitterrace_types::api::Signature;
    const DAY: i64 = 86_400;
    let now = 40 * DAY;
    // A week in the order commits land: every third one was written two days before it landed
    // (rebased), the rest the moment they landed.
    let sig = |time| Signature { email: String::new(), name: String::new(), offset: 0, time };
    let mut list = Vec::new();
    for (i, landed) in (0..7 * 24).map(|h| now - h * 3600).enumerate() {
        let wrote = if i % 3 == 0 { landed - 2 * DAY } else { landed };
        list.push(CommitSummary {
            id: landed.to_string(),
            parents: Vec::new(),
            author: sig(wrote),
            committer: sig(landed),
            subject: String::new(),
        });
    }
    let labels: Vec<String> =
        crate::views::day_groups(&list, now).into_iter().map(|g| g.label).collect();
    let mut seen = std::collections::HashSet::new();
    assert!(labels.iter().all(|l| seen.insert(l.clone())), "{labels:?}");
}

#[test]
fn initials_follow_the_naming_rules() {
    for (name, want) in [
        ("Timo Runge", "TR"),
        ("Ana María López", "AL"),
        ("dependabot", "D"),
        ("dependabot[bot]", "D"),
        ("renovate [bot]", "R"),
        ("x@y.z", "X"),
        ("Tim @home", "T"),
        ("éric dupont", "ÉD"),
        ("e\u{301}va li", "E\u{301}L"),
        ("山田太郎", "山"),
        ("  ", "?"),
        ("", "?"),
        ("[bot]", "?"),
    ] {
        assert_eq!(crate::views::initials(name), want, "{name:?}");
    }
}
