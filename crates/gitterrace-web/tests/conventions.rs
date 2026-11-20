//! AGENTS.md rules that a grep can hold: they fail here instead of in review.

use std::{fs, path::Path};

/// Every image in a template needs alt text, empty for decoration.
#[test]
fn template_images_have_alt() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    for entry in fs::read_dir(dir).expect("templates dir").flatten() {
        let Ok(html) = fs::read_to_string(entry.path()) else {
            continue;
        };
        for tag in html.split("<img").skip(1) {
            let tag = tag.split('>').next().unwrap_or_default();
            assert!(tag.contains("alt="), "{} has an <img> without alt", entry.path().display());
        }
    }
}

/// Every copy goes through `gtCopy` in app.js, which flashes the button and shows the toast. A
/// script that writes the clipboard itself copies silently, and the user cannot tell it worked.
#[test]
fn clipboard_writes_go_through_gt_copy() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).expect("static dir").flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "js") || path.ends_with("app.js") {
            continue;
        }
        let text = fs::read_to_string(&path).expect("read script");
        checked += 1;
        for needle in ["clipboard.writeText", "execCommand(\"copy\")", "execCommand('copy')"] {
            assert!(
                !text.contains(needle),
                "{} calls {needle}: copy through window.gtCopy so the copy shows an indicator",
                path.display()
            );
        }
    }
    assert!(checked > 0, "no scripts found in {}", dir.display());
}

/// Nouns that must never follow a number fixed in the plural: `{{ n }} commits` says
/// "1 commits", `{n} hours` says "1 hours". Render the pair with `fmt::count` or `fmt::noun`
/// (cards: `svg::count`). Time units count too.
const COUNTED_NOUNS: [&str; 26] = [
    "commits",
    "authors",
    "files",
    "lines",
    "repositories",
    "contributors",
    "changes",
    "matches",
    "branches",
    "tags",
    "releases",
    "people",
    "hunks",
    "results",
    "times",
    "seconds",
    "minutes",
    "hours",
    "days",
    "nights",
    "weekdays",
    "weekends",
    "weeks",
    "months",
    "years",
    "streaks",
];

/// The counted noun at the start of `s` after optional spaces, if any.
fn counted_noun(s: &str) -> Option<&str> {
    let word = s.trim_start_matches([' ', '\t', '\n']);
    if word.len() == s.len() {
        return None;
    }
    let end = word.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(word.len());
    COUNTED_NOUNS.contains(&&word[..end]).then(|| &word[..end])
}

/// `}}` or `}}</b>`, whitespace, then a counted noun, as a word.
fn fixed_plural(html: &str) -> Option<String> {
    let mut rest = html;
    while let Some(at) = rest.find("}}") {
        rest = &rest[at + 2..];
        let tail = rest.strip_prefix("</b>").unwrap_or(rest);
        if let Some(noun) = counted_noun(tail) {
            return Some(format!("}}}} {noun}"));
        }
    }
    None
}

/// In Rust source: a format placeholder (`{n}`, `{}`, `{x:>3}`) then a counted noun, as in
/// `format!("{n} hours")`. In JS: a closing `+ "` then a counted noun, as in `n + " hours"`.
fn fixed_plural_in_code(src: &str) -> Option<String> {
    for line in src.lines() {
        let code = line.trim_start();
        if code.starts_with("//") || code.starts_with("assert") {
            continue;
        }
        let mut rest = line;
        while let Some(at) = rest.find('}') {
            let before = &rest[..at];
            rest = &rest[at + 1..];
            // `{}`/`{name}`/`{n:>3}` only: not `}}` escapes, not a closing block brace.
            let open = before.rfind('{');
            let placeholder = open.is_some_and(|o| {
                before[o + 1..].chars().all(|c| c.is_ascii_alphanumeric() || "_.:<>^".contains(c))
            });
            if placeholder
                && !rest.starts_with('}')
                && let Some(noun) = counted_noun(rest)
            {
                return Some(format!("{{..}} {noun}: {}", line.trim()));
            }
        }
        for quote in ["+ \"", "+ '"] {
            let mut rest = line;
            while let Some(at) = rest.find(quote) {
                rest = &rest[at + quote.len()..];
                if let Some(noun) = counted_noun(rest) {
                    return Some(format!("+ \" {noun}: {}", line.trim()));
                }
            }
        }
    }
    None
}

/// The text sources a page is built from: web Rust, and the static JS.
/// An unreadable directory yields nothing, so the test below asserts it found sources.
fn code_sources() -> Vec<std::path::PathBuf> {
    let web = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for dir in [web.join("src"), web.join("static")] {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = entry.path();
            if p.extension().is_some_and(|e| e == "rs" || e == "js") {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn code_never_pairs_a_number_with_a_fixed_plural() {
    let sources = code_sources();
    assert!(sources.len() > 20, "found only {} sources", sources.len());
    for path in sources {
        let Ok(src) = fs::read_to_string(&path) else {
            continue;
        };
        // Unit tests at the bottom may spell out expected text; the guard is for page text.
        let src = src.split("#[cfg(test)]").next().unwrap_or(&src);
        if let Some(hit) = fixed_plural_in_code(src) {
            panic!(
                "{} builds `{hit}`: use fmt::count / fmt::noun (cards: svg::count)",
                path.display()
            );
        }
    }
}

#[test]
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "the inputs are source text the scan reads, not format strings"
)]
fn the_code_plural_scan_sees_what_it_should() {
    assert!(fixed_plural_in_code(r#"format!("{n} hours")"#).is_some());
    assert!(fixed_plural_in_code(r#"format!("{} times", x)"#).is_some());
    assert!(fixed_plural_in_code(r#"format!("{d:>3} days since")"#).is_some());
    assert!(fixed_plural_in_code(r#"var t = n + " commits";"#).is_some());
    assert!(fixed_plural_in_code(r#"format!("{} in 26 weeks", c)"#).is_none());
    assert!(fixed_plural_in_code(r#"format!("{n} {unit}{} ago", s)"#).is_none());
    assert!(fixed_plural_in_code("if x { y } days").is_none());
    assert!(fixed_plural_in_code(r#"format!("{{n}} commits")"#).is_none());
}

#[test]
fn templates_never_pair_a_number_with_a_fixed_plural() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    for entry in fs::read_dir(dir).expect("templates dir").flatten() {
        let Ok(html) = fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Some(hit) = fixed_plural(&html) {
            panic!(
                "{} renders `{hit}`: pass the counted text from Rust (fmt::count / fmt::noun)",
                entry.path().display()
            );
        }
    }
}

#[test]
fn the_plural_scan_sees_what_it_should() {
    assert!(fixed_plural("{{ n }} commits").is_some());
    assert!(fixed_plural("<b>{{ n }}</b> authors").is_some());
    assert!(fixed_plural("{{ n }} commit").is_none());
    assert!(fixed_plural("{{ t.commits_noun }}").is_none());
    assert!(fixed_plural("{{ a }}\n  <td>files</td>").is_none());
}

/// Templates and modules still writing an h1 of their own, waiting to move onto the page head.
/// `schwarzwald/config/own-heads.json`, a ratchet: it only shrinks, toward empty.
fn own_heads() -> Option<Vec<String>> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schwarzwald/config/own-heads.json");
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
    Some(v["own_heads"].as_array()?.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
}

/// Whether `src` writes an h1 tag outside its test module.
fn writes_h1(src: &str) -> bool {
    let live = src.split("#[cfg(test)]").next().unwrap_or_default();
    live.contains("<h1")
}

/// The page head (`pagehead.rs`, `pagehead.html`) is the only place that writes an h1. A page
/// that writes its own head starts at its own height with its own type, which is the drift the
/// one page anatomy exists to end; the browser check `anatomy` measures what this one names.
#[test]
fn only_the_page_head_writes_an_h1() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let allowed = own_heads().expect("schwarzwald/config/own-heads.json lists own_heads");
    let mut found = Vec::new();
    for (dir, ext) in [("templates", "html"), ("src", "rs")] {
        let mut stack = vec![root.join(dir)];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(&d).expect("dir").flatten() {
                let p = entry.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|e| e.to_str()) != Some(ext) {
                    continue;
                }
                let rel = p.strip_prefix(root).expect("under the crate");
                let rel = rel.to_string_lossy().replace('\\', "/");
                let test_file = rel.ends_with("tests.rs") || rel.contains("/tests/");
                if test_file || rel == "templates/pagehead.html" || rel == "src/pagehead.rs" {
                    continue;
                }
                if writes_h1(&fs::read_to_string(&p).unwrap_or_default()) {
                    found.push(rel);
                }
            }
        }
    }
    found.sort();
    let new: Vec<&String> = found.iter().filter(|f| !allowed.contains(f)).collect();
    assert!(new.is_empty(), "{new:?} write an h1: fill a PageHead (pagehead.rs) instead");
    let moved: Vec<&String> = allowed.iter().filter(|f| !found.contains(f)).collect();
    assert!(
        moved.is_empty(),
        "{moved:?} no longer write an h1: take them off schwarzwald/config/own-heads.json"
    );
}

#[test]
fn the_h1_scan_skips_tests_only() {
    assert!(writes_h1("<h1>x</h1>"));
    assert!(writes_h1("format!(\"<h1 class=x>\")"));
    assert!(!writes_h1("fn a() {}\n#[cfg(test)]\nmod t { \"<h1>\" }"));
    assert!(!writes_h1("<h2>x</h2>"));
}

/// Routes that answer a full page, each with the path the browser check `anatomy` walks for
/// it (`scripts/verify/browser/anatomy-lib.mjs`, `PAGES`). Every other route answers a fragment, a
/// file, JSON, a stream or a redirect, and is named in `NOT_PAGES`.
const PAGE_ROUTES: &[(&str, &str)] = &[
    ("/", "'/'"),
    ("/tree/{rev}/{*path}", "'/tree/HEAD/src'"),
    ("/blob/{rev}/{*path}", "'/blob/HEAD/a.txt'"),
    ("/blame/{rev}/{*path}", "'/blame/HEAD/a.txt'"),
    ("/history/{*path}", "'/history/a.txt'"),
    ("/log/{rev}", "'/log/HEAD'"),
    ("/commit/{id}", "'/commit/HEAD'"),
    ("/compare", "'/compare?"),
    ("/refs", "'/refs'"),
    ("/worktree", "'/worktree'"),
    ("/search", "'/search?"),
    ("/stats", "'/stats'"),
    ("/stats/contributors", "'/stats/contributors'"),
    ("/stats/hotspots", "'/stats/hotspots'"),
    ("/stats/languages", "'/stats/languages'"),
    ("/stats/ownership", "'/stats/ownership'"),
    ("/_settings", "'/_settings'"),
];

/// Routes that render no page of their own: aliases and redirects of a page above, fragments,
/// raw files, JSON, the event stream and form posts. The tag page and the repositories home
/// use the head too, but the check's one-repository fixture has neither.
const NOT_PAGES: &[&str] = &[
    "/tree/{rev}",
    "/tree/{rev}/",
    "/hl/{rev}/{*path}",
    "/raw/{rev}/{*path}",
    "/log",
    "/diff/{rev}/{index}",
    "/tag/{name}",
    "/worktrees",
    "/stashes",
    "/worktree/diff/{*path}",
    // HTML fragments: the hits under a diff page's find box, with no page head.
    "/diff-find/{rev}",
    "/worktree/find",
    "/worktree/raw/{*path}",
    // The blob page's own template, walked at `/blob`; the check's fixture has a clean tree.
    "/worktree/blob/{*path}",
    "/events",
    "/api/files/{rev}",
    "/api/refs",
    "/api/last/{rev}",
    "/api/last/{rev}/{*path}",
    "/api/progress",
    "/go/{*rest}",
    "/_send/targets",
    "/r/{name}/_send/targets",
    "/r/{name}",
    "/r/{name}/",
    "/r/{name}/{*rest}",
    "/api/repos",
    "/_dirty",
    "/_repos",
    "/_settings/file",
    "/_settings/check",
];

/// Class: every route the sitemap lists is either a page the anatomy check walks or named as
/// no page. A new page route fails here until it is added to the check, so no page can skip
/// the one head.
#[test]
fn every_page_route_is_walked_by_the_anatomy_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sitemap = fs::read_to_string(root.join("src/sitemap.rs")).expect("sitemap.rs");
    let check = fs::read_to_string(root.join("../../scripts/verify/browser/anatomy-lib.mjs"))
        .expect("scripts/verify/browser/anatomy-lib.mjs");
    let start = sitemap.find("pub const PAGES").expect("PAGES");
    let body = &sitemap[start..];
    let body = &body[..body.find("\n];").expect("end of PAGES")];
    let routes: Vec<&str> =
        body.split('"').skip(1).step_by(2).filter(|s| s.starts_with('/')).collect();
    for r in &routes {
        let page = PAGE_ROUTES.iter().find(|(p, _)| p == r);
        match page {
            Some((_, walked)) => assert!(
                check.contains(walked),
                "{r} is a page: add {walked} to PAGES in scripts/verify/browser/anatomy-lib.mjs"
            ),
            None => assert!(
                NOT_PAGES.contains(r),
                "{r} is new: add it to PAGE_ROUTES (and the anatomy check) or NOT_PAGES"
            ),
        }
    }
    for (p, _) in PAGE_ROUTES {
        assert!(routes.contains(p), "{p} is no longer routed: drop it here");
    }
}
