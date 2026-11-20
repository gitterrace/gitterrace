//! Every view that offers the action menu gets the same entries, in the same order, with the
//! same labels. Only whether an entry is enabled may differ, and a disabled one says why.

use std::path::Path;

use axum::{body::Body, http::Request};
use gitterrace_testkit::git;
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::actions::actions_tests::config_dir;

/// Several actions over different kinds, plus send targets, so a per-view filter shows.
const MENU: &str = r#"version = 1
[[action]]
id = "lines"
label = "Lines only"
kinds = ["line"]
command = ["/usr/bin/true"]
[[action]]
id = "commit"
label = "Commit only"
kinds = ["commit"]
command = ["/usr/bin/true"]
key = "x"
[[action]]
id = "page"
label = "Any page"
kinds = ["page", "file", "line", "commit", "hunk", "blame"]
command = ["/usr/bin/true"]
[[action]]
id = "file"
label = "Whole file"
kinds = ["file"]
command = ["/usr/bin/true"]
[[send]]
id = "a"
label = "Target A"
url = "https://a.example/new?t={title}"
[[send]]
id = "b"
label = "Target B"
url = "https://b.example/new?t={title}&n={notes}"
"#;

/// One menu entry as the page shows it.
#[derive(Debug, PartialEq, Eq)]
struct Entry {
    label: String,
    /// `None` when enabled, else the tip the disabled entry carries.
    why: Option<String>,
}

fn repo(root: &Path) -> (std::path::PathBuf, String) {
    let p = root.join("menu");
    std::fs::create_dir_all(p.join("src")).expect("dir");
    git(&p, &["init", "-q", "-b", "main"]);
    git(&p, &["config", "user.name", "A"]);
    git(&p, &["config", "user.email", "a@x.y"]);
    std::fs::write(p.join("README.md"), "# Menu\n\nSome prose.\n").expect("w");
    std::fs::write(p.join("src/a.rs"), "fn a() {}\n").expect("w");
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "feat: first"]);
    std::fs::write(p.join("src/a.rs"), "fn a() {}\nfn b() {}\n").expect("w");
    git(&p, &["commit", "-qam", "feat: second"]);
    let sha = git(&p, &["rev-parse", "HEAD"]);
    (p, sha)
}

async fn html(r: &axum::Router, uri: &str) -> String {
    let res = r
        .clone()
        .oneshot(
            Request::get(format!("/r/menu{uri}"))
                .header("host", gitterrace_testkit::on_port("localhost"))
                .body(Body::empty())
                .expect("req"),
        )
        .await
        .expect("res");
    assert_eq!(res.status().as_u16(), 200, "{uri}");
    let b = res.into_body().collect().await.expect("body").to_bytes();
    String::from_utf8_lossy(&b).into_owned()
}

/// The entries `actions.js` builds from the page's `data-menu`, for a selection of `kind`:
/// the same walk the menu makes, kept beside the list it reads.
fn entries(html: &str, kind: &str) -> Vec<Entry> {
    let i = html.find("data-menu=\"").expect("menu meta") + 11;
    let raw = &html[i..i + html[i..].find('"').expect("end")];
    let json = raw
        .replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&");
    let list: Vec<serde_json::Value> = serde_json::from_str(&json).expect("menu json");
    let mut out: Vec<Entry> = ["Copy permalink", "Copy as markdown", "Send to\u{2026}"]
        .iter()
        .map(|l| Entry { label: (*l).to_owned(), why: None })
        .collect();
    for a in list {
        let enabled = a["kinds"].as_array().is_some_and(|k| k.iter().any(|k| k == kind));
        out.push(Entry {
            label: a["label"].as_str().unwrap_or_default().to_owned(),
            why: (!enabled).then(|| a["why"].as_str().unwrap_or_default().to_owned()),
        });
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn every_view_offers_the_same_menu() {
    let tmp = tempfile::tempdir().expect("tmp");
    let (dir, sha) = repo(tmp.path());
    let cfg = config_dir(MENU);
    let r = crate::build_multi(
        vec![dir],
        None,
        Some(&cfg.path().join("actions.toml")),
        &crate::app::Env::new(gitterrace_config::Config::default()),
    )
    .expect("router")
    .0;
    // Each view with the selection its menu acts on.
    let views = [
        ("blob code", "/blob/main/src/a.rs".to_owned(), "file"),
        ("blob code, lines", "/blob/main/src/a.rs".to_owned(), "line"),
        ("blob preview", "/blob/main/README.md".to_owned(), "file"),
        ("blame", "/blame/main/src/a.rs".to_owned(), "blame"),
        ("commit", format!("/commit/{sha}"), "commit"),
        ("commit hunk", format!("/commit/{sha}"), "hunk"),
        ("compare", "/compare?from=main~1&to=main".to_owned(), "hunk"),
        ("tree", "/tree/main".to_owned(), "page"),
        ("log", "/log/main".to_owned(), "page"),
        ("history", "/history/src/a.rs".to_owned(), "page"),
        ("search", "/search?q=fn".to_owned(), "search-hit"),
        ("hotspots", "/stats/hotspots".to_owned(), "insight"),
        ("overview", "/".to_owned(), "page"),
        ("stats", "/stats".to_owned(), "page"),
    ];
    let mut first: Option<(&str, Vec<Entry>)> = None;
    for (name, uri, kind) in &views {
        let got = entries(&html(&r, uri).await, kind);
        for e in &got {
            if let Some(w) = &e.why {
                assert!(
                    w.starts_with("Works on "),
                    "{name}: {} disabled without a reason",
                    e.label
                );
            }
        }
        let shape: Vec<&str> = got.iter().map(|e| e.label.as_str()).collect();
        match &first {
            None => first = Some((name, got)),
            Some((n0, e0)) => {
                let s0: Vec<&str> = e0.iter().map(|e| e.label.as_str()).collect();
                assert_eq!(shape, s0, "{name} and {n0} list different entries");
            }
        }
    }
    // The views must not all agree by offering nothing.
    let (_, e0) = first.expect("views");
    assert_eq!(e0.len(), 7, "{e0:?}");
}
