//! Exported Markdown keeps plain heading anchors, so a page opened with JS off still jumps.

const DEFAULT_PAGES: Option<usize> = None;

/// Guard class: a static page whose in-page links only resolve through script. Every
/// `href="#x"` in every exported page must name an `id="x"` on the same page, and the README
/// heading is reached as `#reference`. Red when ids and hrefs are rewritten apart (the old
/// `user-content-` prefix on ids only) or when a target needs nav.js to be found.
#[tokio::test]
async fn exported_fragments_resolve_without_script() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fx = gitterrace_testkit::Fixture::init(&dir.path().join("r"))
        .write(
            "README.md",
            "# Title\n\n[ref](#reference) note[^1]\n\n## Reference\n\nText.\n\n[^1]: a note\n",
        )
        .commit("readme");
    let out = dir.path().join("out");
    let scope = crate::export::Scope {
        commits: 5,
        max_file: 64 * 1024,
        projects: Vec::new(),
        force: false,
        config: gitterrace_config::Config::default(),
    };
    crate::export::export(&[fx.path().to_path_buf()], &out, &scope, DEFAULT_PAGES)
        .await
        .expect("export");
    let values = |html: &str, attr: &str| -> Vec<String> {
        html.split(attr)
            .skip(1)
            .filter_map(|s| s.split('"').next().map(str::to_owned))
            .collect()
    };
    let mut stack = vec![out.clone()];
    let mut seen_reference = false;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "html") {
                continue;
            }
            let html = std::fs::read_to_string(&p).expect("read");
            if !html.contains("id=\"reference\"") {
                continue;
            }
            seen_reference = true;
            let ids = values(&html, " id=\"");
            assert!(html.contains("href=\"#reference\""), "{}", p.display());
            assert!(!html.contains("user-content-"), "{}", p.display());
            for h in values(&html, "href=\"#") {
                assert!(ids.contains(&h), "{}: #{h} has no target", p.display());
            }
        }
    }
    assert!(seen_reference, "no exported page renders the README");
}
