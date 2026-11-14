//! The site's prose matches what was built: no document calls a multi-page site one page.

use crate::site::tests::generated;

/// Prose files that describe the site: the README, the documents and the xtask sources.
fn site_prose(root: &std::path::Path) -> Vec<(String, String)> {
    let mut files = vec!["README.md".to_owned()];
    for dir in ["docs", "xtask/src"] {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "md" || x == "rs") {
                    // `/` on every system, so the suffix checks below match on Windows too.
                    let rel = p
                        .strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    files.push(rel);
                }
            }
        }
    }
    files.sort();
    files
        .into_iter()
        .filter_map(|f| Some((f.clone(), std::fs::read_to_string(root.join(&f)).ok()?)))
        .collect()
}

/// Lines that call the site a one-page or single-page site while it has `pages` pages.
fn one_page_claims(pages: usize, prose: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    if pages < 2 {
        return bad;
    }
    for (file, body) in prose {
        // This guard names the phrases it looks for.
        if file.ends_with("site/prose.rs") {
            continue;
        }
        for (n, line) in body.lines().enumerate() {
            let l = line.to_lowercase().replace('-', " ");
            if [
                "one page site",
                "single page site",
                "one page website",
                "single page website",
            ]
            .iter()
            .any(|p| l.contains(p))
            {
                bad.push(format!(
                    "{file}:{}: calls the site one page, but it has {pages} pages",
                    n + 1
                ));
            }
        }
    }
    bad
}

/// The class this guards: prose that still calls the site a one-page site after it grew.
#[test]
fn no_prose_calls_the_multi_page_site_one_page() {
    let (pages, _) = generated();
    let html = pages
        .iter()
        .filter(|p| {
            std::path::Path::new(&p.0)
                .extension()
                .is_some_and(|x| x == "html")
        })
        .count();
    let bad = one_page_claims(html, &site_prose(&crate::repo_root()));
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_planted_one_page_claim_is_reported() {
    let prose = vec![(
        "README.md".to_owned(),
        "The one-page site is here.\n".to_owned(),
    )];
    assert!(one_page_claims(1, &prose).is_empty());
    let bad = one_page_claims(5, &prose).join("\n");
    assert!(bad.contains("README.md:1"), "{bad}");
}
