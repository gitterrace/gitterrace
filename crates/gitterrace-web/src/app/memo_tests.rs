//! The memo never serves a page with a stale live version.

use std::sync::atomic::Ordering;

use crate::wiring_tests::{app, repo};

#[test]
fn a_memo_hit_carries_the_version_of_now() {
    let dir = tempfile::tempdir().expect("tmp");
    let a = app(&repo(dir.path(), "r"), gitterrace_config::Config::default());
    let key = a.memo_key("stats", Some("q=type:fix"));
    let page = |v: u64| format!("<body data-live=\"notice\" data-live-v=\"{v}\">x</body>");
    a.memo_put(key.clone(), &page(a.version()));
    // A change the key does not see (staging into a tree already dirty): only the version moves.
    let next = a.version().wrapping_add(7);
    a.version.store(next, Ordering::Release);
    assert_eq!(
        a.memo_key("stats", Some("q=type:fix")),
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

/// The memo survives the idle release, so what it keeps must be small: pages of tens of KB
/// held as is pinned multi-page slabs on 64 KB page kernels. A rendered page is repetitive
/// markup, so it must sit in the memo at a fraction of its size and come back byte for byte.
#[test]
fn the_memo_keeps_a_page_at_a_fraction_of_its_size() {
    let page = (0..400)
        .map(|i| {
            format!("<tr><td class=\"name\"><a href=\"/c/{i:040x}\">fix: item {i}</a></td></tr>\n")
        })
        .collect::<Vec<_>>()
        .concat();
    let packed = super::Packed::pack(&page).expect("pack");
    assert!(
        crate::bounded::Weight::weight(&packed) * 4 < page.len(),
        "{} bytes kept for a {} byte page",
        crate::bounded::Weight::weight(&packed),
        page.len()
    );
    assert_eq!(packed.unpack().as_deref(), Some(page.as_str()));
}
