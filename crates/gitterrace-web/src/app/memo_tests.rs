//! The memo never serves a page with a stale live version.

use std::sync::{Arc, atomic::Ordering};

use crate::wiring_tests::{app, repo};

#[test]
fn a_memo_hit_carries_the_version_of_now() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = app(&repo(dir.path(), "r"), gitterrace_config::Config::default());
    let key = a.memo_key("stats", Some("type=fix"));
    let page = |v: u64| format!("<body data-live=\"notice\" data-live-v=\"{v}\">x</body>");
    a.memo_put(key.clone(), Arc::new(page(a.version())));
    // A change the key does not see (staging into a tree already dirty): only the version moves.
    let next = a.version().wrapping_add(7);
    a.version.store(next, Ordering::Release);
    assert_eq!(
        a.memo_key("stats", Some("type=fix")),
        key,
        "the key stays put"
    );
    assert_eq!(a.memo_get(&key).as_deref(), Some(page(next).as_str()));
}

#[test]
fn a_page_without_a_live_version_is_served_as_is() {
    assert_eq!(super::stamp("<p>plain</p>", 3), "<p>plain</p>");
    assert_eq!(super::stamp("data-live-v=\"1", 3), "data-live-v=\"1");
}

/// Every read of the page memo goes through `App::memo_get`, which stamps the version: no other
/// file reads the memo map itself, so no route can hand out a cached page unstamped.
#[test]
fn only_memo_get_reads_the_page_memo() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut bad = Vec::new();
    let mut stack = vec![src];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs")
                && !p.ends_with("app/memo.rs")
                && !p.ends_with("app/memo_tests.rs")
            {
                let text = std::fs::read_to_string(&p).expect("read");
                for (n, l) in text.lines().enumerate() {
                    if l.contains("memo.lock()") && l.contains(".get(") {
                        bad.push(format!("{}:{}", p.display(), n + 1));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "read the memo through App::memo_get: {bad:?}"
    );
}
