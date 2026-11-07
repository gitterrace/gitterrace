//! Every commit id on a page is the shared `a.sha` link. Renders the main pages of a fixture
//! repository and checks that no 7-to-40 hex token in visible text sits outside one.

use gitterrace_testkit::{git, page as get};

/// Hex runs of 7 to 40 digits in visible text (outside tags, scripts, styles and `a.sha`),
/// bounded by non-word characters and holding both a digit and a letter, so counts and plain
/// words are not mistaken for ids.
fn bare_ids(html: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = html;
    let mut in_sha = false;
    let mut skip_until: Option<&str> = None;
    let mut text = String::new();
    while let Some(lt) = rest.find('<') {
        if skip_until.is_none() && !in_sha {
            text.push_str(&rest[..lt]);
            text.push('\n');
        }
        rest = &rest[lt..];
        let Some(gt) = rest.find('>') else { break };
        let tag = &rest[1..gt];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_ascii_lowercase();
        if let Some(end) = skip_until {
            if tag.starts_with('/') && name == end {
                skip_until = None;
            }
        } else if tag.starts_with('/') {
            if name == "a" || name == "button" {
                in_sha = false;
            }
        } else if name == "script" {
            skip_until = Some("script");
        } else if name == "style" {
            skip_until = Some("style");
        } else if (name == "a" && tag.contains("class=\"sha\""))
            || (name == "button" && tag.contains("id=\"refbtn\""))
        {
            // The ref switcher shows the short id of a pinned commit as its label.
            in_sha = true;
        }
        rest = &rest[gt + 1..];
    }
    if skip_until.is_none() && !in_sha {
        text.push_str(rest);
    }
    let bytes = text.as_bytes();
    let word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut i = 0;
    while i < bytes.len() {
        if !word(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && word(bytes[i]) {
            i += 1;
        }
        let tok = &text[start..i];
        let hex = tok.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        let mixed =
            tok.bytes().any(|b| b.is_ascii_digit()) && tok.bytes().any(|b| b.is_ascii_lowercase());
        if hex && mixed && (7..=40).contains(&tok.len()) {
            found.push(tok.to_owned());
        }
    }
    found
}

/// Positive control: the markup an older commit page had, a parent id as plain code in a
/// link, is flagged, and the shared link is not.
#[test]
fn scanner_flags_a_plain_id() {
    let old = r#"<p>parent <a href="/commit/abc1234def"><code>abc1234d</code></a></p>"#;
    assert_eq!(bare_ids(old), vec!["abc1234d".to_owned()]);
    let new = concat!(
        r#"<p>parent <a class="sha" href="/commit/abc1234def" "#,
        r#"data-tip="abc1234def">abc1234d</a></p>"#,
    );
    assert!(bare_ids(new).is_empty());
    let script = "<script>var x = 'abc1234d';</script>";
    assert!(bare_ids(script).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn every_commit_id_is_a_sha_link() {
    let root = tempfile::tempdir().expect("tmp");
    let p = root.path().join("repo");
    std::fs::create_dir_all(p.join("src")).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    std::fs::write(p.join("README.md"), "# Hi\n").expect("w");
    std::fs::write(p.join("src/a.rs"), "fn main() {}\n").expect("w");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "feat(core): first"]);
    git(&p, &["branch", "side"]);
    git(&p, &["tag", "-a", "v1", "-m", "release one"]);
    std::fs::write(p.join("src/a.rs"), "fn main() {\n}\n").expect("w");
    git(&p, &["commit", "-qam", "fix: second"]);
    std::fs::write(p.join("src/a.rs"), "fn main() {\n    1;\n}\n").expect("w");
    git(&p, &["stash", "push", "-q", "-m", "wip stash"]);
    let wt = root.path().join("wt");
    git(
        &p,
        &["worktree", "add", "-q", wt.to_str().expect("utf8"), "side"],
    );
    // A detached HEAD puts the id into the banner shown on every page.
    git(&p, &["checkout", "-q", "--detach"]);

    let router = gitterrace_web::app_router(&p).expect("router");
    let head = git(&p, &["rev-parse", "HEAD"]);

    // The index builds in the background: wait until commit search has rows.
    gitterrace_testkit::wait::page_until(&router, "/search?q=first&kind=commits", |b| {
        b.contains("class=\"crow\"")
    })
    .await;

    let pages = [
        "/".to_owned(),
        "/log/HEAD".to_owned(),
        format!("/commit/{head}"),
        "/search?q=first&kind=commits".to_owned(),
        "/history/src/a.rs".to_owned(),
        "/compare?base=side&head=main".to_owned(),
        "/blame/HEAD/src/a.rs".to_owned(),
        "/refs".to_owned(),
        "/tag/v1".to_owned(),
        "/stats".to_owned(),
        "/stats/languages".to_owned(),
        "/stats/hotspots".to_owned(),
    ];
    let mut with_links = 0;
    for uri in &pages {
        let (status, body) = get(&router, uri).await;
        assert_eq!(status, 200, "{uri}");
        let bare = bare_ids(&body);
        assert!(bare.is_empty(), "{uri}: commit ids outside a.sha: {bare:?}");
        if body.contains("class=\"sha\"") {
            with_links += 1;
        }
    }
    assert!(
        with_links >= 8,
        "the fixture should show ids on most pages, got {with_links}"
    );
}
