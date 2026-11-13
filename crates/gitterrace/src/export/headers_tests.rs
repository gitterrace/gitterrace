//! The `_headers` file a static host reads, and the build version every exported asset URL carries.

/// The budget an export gets with no flag and no config: no cap.
const DEFAULT_PAGES: Option<usize> = None;

fn scope(force: bool) -> crate::export::Scope {
    crate::export::Scope {
        commits: 3,
        max_file: 1024,
        projects: Vec::new(),
        force,
        config: gitterrace_config::Config::default(),
    }
}

#[test]
fn headers_file_carries_the_page_policy() {
    let h = crate::export::headers_file();
    assert!(h.contains("font-src 'self' data:"), "{h}");
    assert!(
        !gitterrace_web::PAGE_CSP.contains("font-src"),
        "the served app takes no data: fonts"
    );
    assert!(!h.contains("script-src 'self' 'unsafe-inline'"));
}

/// A published export revalidates its pages like the server does, and the page rule stays off
/// `/*`, where a host would merge it into the static lifetime.
#[test]
fn headers_file_revalidates_pages_but_not_static() {
    let h = crate::export::headers_file();
    for rule in ["/\n", "/r/*\n"] {
        let at = h
            .find(&format!("\n{rule}"))
            .unwrap_or_else(|| panic!("{rule:?} in {h}"));
        let block = h[at + 1..].split("\n\n").next().unwrap_or_default();
        assert!(block.contains("Cache-Control: no-cache"), "{block}");
    }
    let top = h.split("\n\n").next().unwrap_or_default();
    assert!(!top.contains("Cache-Control"), "{top}");
}

/// Guard class: a static host (Cloudflare, Netlify) takes one `*` per rule and ignores a
/// rule with two, so a raw file would lose its sandbox. Every rule path has at most one.
#[test]
fn every_header_rule_has_at_most_one_splat() {
    let h = crate::export::headers_file();
    let rules: Vec<&str> = h.lines().filter(|l| l.starts_with('/')).collect();
    assert!(rules.iter().any(|r| r.contains("/raw/")), "{h}");
    for r in rules {
        assert!(r.matches('*').count() <= 1, "{r} has more than one *");
    }
}

/// Guard class: `_headers` keeps `/static/*` a year, so a page naming an asset without the
/// build's `?v=` would pair the next export's pages with this export's scripts. Every
/// script, stylesheet and font an exported page or stylesheet names carries the version.
#[tokio::test]
async fn every_asset_an_exported_page_names_carries_the_build_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = gitterrace_testkit::Fixture::standard(&dir.path().join("r"));
    let out = dir.path().join("out");
    crate::export::export(
        &[repo.path().to_path_buf()],
        &out,
        &scope(false),
        DEFAULT_PAGES,
    )
    .await
    .expect("export");
    let v = format!("?v={}", gitterrace_web::asset_version());
    let mut stack = vec![out.clone()];
    let mut seen = 0;
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir") {
            let p = e.expect("entry").path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if !p.extension().is_some_and(|x| x == "html" || x == "css") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("read");
            for (at, _) in text.match_indices("static/") {
                let rest = &text[at..];
                let end = rest.find(['"', '\'', ')', ' ', '>']).unwrap_or(rest.len());
                let url = &rest[..end];
                // A folder name in prose or a comment, not a file.
                if !url.contains('.') {
                    continue;
                }
                seen += 1;
                assert!(url.ends_with(&v), "{}: {url} lacks {v}", p.display());
            }
        }
    }
    assert!(seen > 5, "the export names its assets");
}
