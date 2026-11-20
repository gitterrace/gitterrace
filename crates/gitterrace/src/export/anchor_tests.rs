//! Links in an export work without a server: Markdown keeps plain heading anchors, so a page
//! opened with JS off still jumps, and every link reaches a file the export wrote.

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
        html.split(attr).skip(1).filter_map(|s| s.split('"').next().map(str::to_owned)).collect()
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

/// Class: a link in an export that a static host cannot serve. Every `href`/`src` on every
/// written page, read back from disk, reaches a file the export wrote, and no link asks a page
/// for a query: a static file ignores it and shows the unfiltered page. Covers links to pages
/// out of scope (another branch's log, a file at an older commit, blame) and filter links
/// alike, whatever template writes them.
#[tokio::test]
async fn every_exported_link_reaches_a_written_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = gitterrace_testkit::Fixture::standard(&dir.path().join("r"));
    let out = dir.path().join("out");
    let scope = crate::export::Scope {
        commits: 2,
        max_file: 64 * 1024,
        projects: Vec::new(),
        force: false,
        config: gitterrace_config::Config::default(),
    };
    crate::export::export(&[repo.path().to_path_buf()], &out, &scope, DEFAULT_PAGES)
        .await
        .expect("export");
    let mut dead = Vec::new();
    let mut pages = 0;
    let mut todo = vec![out.clone()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
                continue;
            }
            if p.extension().is_none_or(|x| x != "html") {
                continue;
            }
            pages += 1;
            let html = std::fs::read_to_string(&p).expect("read");
            let base = p.parent().expect("parent");
            for m in [" href=\"", " src=\""] {
                for part in html.split(m).skip(1) {
                    let link = part.split('"').next().unwrap_or("").replace("&amp;", "&");
                    if link.is_empty() || link.starts_with('#') || link.contains(':') {
                        continue;
                    }
                    let (path, tail) = link.split_at(link.find(['?', '#']).unwrap_or(link.len()));
                    let file = base.join(gitterrace_git::percent_decode(path));
                    let page = file.file_name().is_some_and(|f| f == "index.html");
                    let real = file.canonicalize().ok().filter(|f| f.is_file());
                    let inside = real
                        .as_ref()
                        .is_some_and(|f| f.starts_with(out.canonicalize().expect("out")));
                    if !inside || (page && tail.starts_with('?')) {
                        dead.push(format!(
                            "{} -> {link}",
                            p.strip_prefix(&out).unwrap_or(&p).display()
                        ));
                    }
                }
            }
        }
    }
    assert!(pages > 10, "pages were written");
    dead.sort();
    dead.dedup();
    assert!(
        dead.is_empty(),
        "{} dead links, first: {:#?}",
        dead.len(),
        &dead[..dead.len().min(20)]
    );
}
