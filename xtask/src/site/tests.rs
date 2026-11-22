use super::shots::SHOT_DIR;
use super::*;

/// The built landing page, from the real sources.
fn landing() -> String {
    let (pages, _) = generated();
    pages.into_iter().find(|p| p.0 == "index.html").map(|p| p.1).expect("landing page")
}

/// One figure per gallery and hand-off shot, each one picture whose dark source follows the
/// system, with its own close link: no second copy of the image for the overlay.
#[test]
fn gallery_has_one_figure_per_shot_and_a_close_link_each() {
    let page = landing();
    let shown = ["insights", "ownership", "commit", "blame", "narrow", "send", "agent"];
    assert_eq!(page.matches("<figure ").count(), shown.len());
    assert_eq!(page.matches("class=\"x\"").count(), shown.len());
    for name in shown {
        assert!(page.contains(&format!("id=\"shot-{name}\"")), "{name}");
        assert_eq!(page.matches(&format!("screenshots/{name}-light.thumb.webp")).count(), 1);
        assert_eq!(page.matches(&format!("screenshots/{name}-dark.thumb.webp")).count(), 1);
    }
    assert!(!page.contains("class=\"lightbox\""));
}

/// Only the hero loads at once, and it is the one marked high priority.
#[test]
fn only_the_hero_picture_is_eager() {
    let page = landing().replace('\n', " ");
    let eager = page.matches("<img ").count() - page.matches("loading=\"lazy\"").count();
    assert_eq!(eager, 1);
    assert!(page.contains("fetchpriority=\"high\""));
}

/// The hero names the brew command first with `git terrace` under it, and the buttons go to
/// the install section, the demo and the repository.
#[test]
fn the_hero_shows_brew_and_git_terrace_above_the_fold() {
    let page = landing();
    let hero = page.split_once("<div class=\"trust\"").map_or("", |h| h.0);
    let plain = hero.split('<').map(|s| s.split_once('>').map_or(s, |x| x.1)).collect::<String>();
    let brew = plain.find("brew install").expect("brew in hero");
    assert!(plain[brew..].contains("git terrace"));
    assert!(hero.contains("href=\"#install\""));
    assert!(hero.contains(">GitHub</a>"));
}

/// The hand-off section shows the MCP registration the docs name.
#[test]
fn the_hand_off_names_the_mcp_command() {
    assert!(landing().contains("git terrace mcp --served"));
}

#[test]
fn redirect_target_is_the_installer_url() {
    let r = redirects("https://github.com/o/r", "r");
    assert!(r.contains(
        "/install.sh https://github.com/o/r/releases/latest/download/r-installer.sh 302"
    ));
}

/// A VP8X header of the given canvas size, enough for `webp_size`.
fn webp(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"RIFF\0\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
    b.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
    b.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
    b
}

#[test]
fn a_missing_unreferenced_or_wrongly_sized_screenshot_fails() {
    let dir = std::env::temp_dir().join(format!("gt-shots-{}", std::process::id()));
    let shots = dir.join(SHOT_DIR);
    let site = shots.clone();
    std::fs::create_dir_all(&shots).expect("mkdir");
    let err = check_shots(&dir).expect_err("no webp fails").to_string();
    assert!(err.contains("is missing (make site-shots)"), "{err}");
    for (n, ..) in SHOTS {
        for c in ["light", "dark"] {
            let (thumb, full) = sizes(n);
            std::fs::write(site.join(format!("{n}-{c}.webp")), webp(full.0, full.1))
                .expect("write");
            std::fs::write(site.join(format!("{n}-{c}.thumb.webp")), webp(thumb.0, thumb.1))
                .expect("write");
        }
    }
    check_shots(&dir).expect("complete set passes");
    std::fs::write(site.join("blame-dark.thumb.webp"), webp(1200, 750)).expect("write");
    let err = check_shots(&dir).expect_err("wrong size fails").to_string();
    assert!(err.contains("blame-dark.thumb.webp is 1200x750"), "{err}");
    assert!(err.contains("make site-shots"), "{err}");
    std::fs::write(site.join("blame-dark.thumb.webp"), webp(1280, 800)).expect("write");
    // A PNG beside them is a leftover of the old README copy: refused.
    std::fs::write(site.join("blame-dark.png"), b"x").expect("write");
    let err = check_shots(&dir).expect_err("a png fails").to_string();
    assert!(err.contains("not referenced"), "{err}");
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

/// Every heading of an internals doc is an id on its page, so a link can name the rule.
#[test]
fn internals_pages_give_every_heading_its_slug_as_id() {
    let text = "# Title\n\n## Code age and releases\n\ntext\n";
    assert_eq!(docs::anchors(text), ["title", "code-age-and-releases"]);
    let links = docs::Links::new("https://r");
    let html =
        docs::body(text, "docs/internals/x.md", &links, &mut |_, h| docs::slug(h)).expect("body");
    assert!(html.contains("<h2 id=\"code-age-and-releases\">Code age and releases</h2>"));
    assert_eq!(docs::page_path("architecture"), "docs/architecture/");
}

/// The install tabs show every way, brew first and open, and each way has a radio, a label
/// and a panel the CSS can reach.
#[test]
fn install_tabs_pair_every_radio_with_a_panel() {
    for way in ["curl", "brew", "cargo"] {
        assert!(PAGE_CSS.contains(&format!("#install-{way}:checked~#panel-{way}")));
    }
}

/// Every relative link, image and `#anchor` of the generated pages that does not resolve to a
/// generated file or to an id on the page it names. `pages` is (site path, content).
pub(super) fn unresolved(pages: &[(String, String)], files: &[String]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        let dir = path.rsplit_once('/').map_or("", |d| d.0);
        for attr in ["href", "src"] {
            for dest in attrs(html, attr) {
                if dest.contains("://") || dest.starts_with("mailto:") {
                    continue;
                }
                let (file, anchor) = dest.split_once('#').unwrap_or((dest, ""));
                // An asset's `?v=` is its content hash, not another file.
                let file = file.split('?').next().unwrap_or(file);
                let target = if file.is_empty() {
                    path.clone()
                } else {
                    let abs = file.strip_prefix('/').map_or_else(
                        || crate::docs_index::normalize(&Path::new(dir).join(file)),
                        str::to_owned,
                    );
                    // normalize drops a trailing `/`, so a directory link is read off `file`.
                    if abs.is_empty() || abs.ends_with('/') {
                        format!("{abs}index.html")
                    } else if file.ends_with('/') {
                        format!("{abs}/index.html")
                    } else {
                        abs
                    }
                };
                if !files.contains(&target) {
                    bad.push(format!("{path}: {dest}: no generated file {target}"));
                } else if !anchor.is_empty() {
                    let ids = pages.iter().find(|p| p.0 == target).map(|p| &p.1);
                    if ids.is_some_and(|h| !attrs(h, "id").contains(&anchor)) {
                        bad.push(format!("{path}: {dest}: no id {anchor} in {target}"));
                    }
                }
            }
        }
    }
    bad
}

/// `path` relative to the built site.
fn site_rel(path: &str) -> &str {
    path.strip_prefix(SITE_DIR).and_then(|p| p.strip_prefix('/')).unwrap_or(path)
}

/// The generated site, as (site path, content) of every page and the paths of every file.
pub(super) fn generated() -> (Vec<(String, String)>, Vec<String>) {
    let root = crate::repo_root();
    let ws = crate::facts::load(&root).expect("facts");
    let outs = render(&root, &ws).expect("render");
    let mut pages = Vec::new();
    let mut files = Vec::new();
    for out in outs {
        match out {
            Out::File { path, body } => {
                let rel = site_rel(&path).to_owned();
                if std::path::Path::new(&rel).extension().is_some_and(|e| e == "html") {
                    pages.push((rel.clone(), body));
                }
                files.push(rel);
            }
            Out::Bytes { path, .. } => {
                files.push(site_rel(&path).to_owned());
            }
            Out::Block { .. } => {}
        }
    }
    (pages, files)
}

/// The class this guards: a page that links to a page, file or heading the site does not have.
#[test]
fn every_link_and_anchor_of_every_page_resolves() {
    let (pages, files) = generated();
    assert!(pages.len() > 10, "{} pages", pages.len());
    let bad = unresolved(&pages, &files);
    assert!(bad.is_empty(), "unresolved:\n{}", bad.join("\n"));
}

/// The class this guards: two elements of one page with the same id.
#[test]
fn no_generated_page_repeats_an_id() {
    let (pages, _) = generated();
    for (path, html) in &pages {
        docs::check_unique_ids(path, html).expect("unique ids");
    }
}

#[test]
fn a_planted_duplicate_heading_id_is_reported() {
    let html = "<h2 id=\"a\">A</h2><h3 id=\"b\">B</h3><h2 id=\"a\">A</h2>";
    let err = docs::check_unique_ids("docs/x/", html).expect_err("duplicate").to_string();
    assert!(err.contains("docs/x/") && err.contains("id a"), "{err}");
    assert!(docs::check_unique_ids("docs/x/", "<h2 id=\"a\"></h2>").is_ok());
}

#[test]
fn a_planted_bad_link_or_anchor_is_reported() {
    let (mut pages, files) = generated();
    pages[0].1.push_str(
        "<a href=\"/docs/nope/#x\">a</a><a href=\"/docs/why/#nope\">b</a><a href=\"#nope\">c</a>",
    );
    let bad = unresolved(&pages, &files).join("\n");
    assert!(bad.contains("/docs/nope/#x: no generated file docs/nope/index.html"), "{bad}");
    assert!(bad.contains("#nope: no id nope in docs/why/index.html"), "{bad}");
    assert!(bad.contains("#nope: no id nope in "), "{bad}");
}

/// The class this guards: a generated page a keyboard or screen reader user cannot skip into or
/// find the title of. Every page opens its body with a skip link to `#main` and has one h1.
#[test]
fn every_page_has_a_skip_link_first_and_one_h1() {
    let (pages, _) = generated();
    let mut bad = Vec::new();
    for (path, html) in &pages {
        let body = html.split_once("<body>\n").map_or("", |b| b.1);
        if !body.starts_with("<a class=\"skip\" href=\"#main\">") {
            bad.push(format!("{path}: the body does not open with the skip link"));
        }
        if !html.contains("<main id=\"main\"") {
            bad.push(format!("{path}: no main with the id main"));
        }
        let h1 = html.matches("<h1").count();
        if h1 != 1 {
            bad.push(format!("{path}: {h1} h1"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The hosts of the maker's other sites. The site links none of them: each stands on its own.
const SIBLING_HOSTS: [&str; 2] = ["tendir.app", "timorunge.com"];

/// What on `pages` points at a sibling site: a "same fleet" line, or a link to a sibling host.
fn sibling_problems(pages: &[(String, String)]) -> Vec<String> {
    let mut bad = Vec::new();
    for (path, html) in pages {
        if html.to_lowercase().contains("same fleet") {
            bad.push(format!("{path}: names the sibling sites"));
        }
        for href in attrs(html, "href") {
            let host = href.split("://").nth(1).and_then(|r| r.split('/').next());
            if let Some(host) = host
                && SIBLING_HOSTS.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
            {
                bad.push(format!("{path}: links the sibling site {href}"));
            }
        }
    }
    bad
}

/// The class this guards: a page that cross-links the maker's other sites, by a footer line or
/// by any link, on any template.
#[test]
fn no_page_links_a_sibling_site() {
    let (pages, _) = generated();
    let bad = sibling_problems(&pages);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

#[test]
fn a_planted_sibling_line_or_link_is_reported() {
    let pages = vec![
        ("a".to_owned(), "<p>Also from the same fleet</p>".to_owned()),
        ("b".to_owned(), "<a href=\"https://www.tendir.app/x\">t</a>".to_owned()),
        ("c".to_owned(), "<a href=\"https://github.com/o/tendir.app\">ok</a>".to_owned()),
    ];
    let bad = sibling_problems(&pages).join("\n");
    assert!(bad.contains("a: names the sibling sites"), "{bad}");
    assert!(bad.contains("b: links the sibling site"), "{bad}");
    assert!(!bad.contains("c:"), "{bad}");
}

/// The class this guards: a docs page whose rail is missing, or shown on a page too short for one.
/// A page with three or more level 2 headings has the app's rail, every other page has none.
#[test]
fn the_rail_is_on_every_docs_page_with_three_h2_and_only_there() {
    let (pages, _) = generated();
    let mut bad = Vec::new();
    for (path, html) in &pages {
        let main = html.split("<main").nth(1).unwrap_or("");
        let main = main.split("</main>").next().unwrap_or("");
        let h2 = main.matches("<h2").count();
        let rail = html.contains("<nav class=\"outline rail\"");
        let want = path.starts_with("docs/") && path != "docs/index.html" && h2 >= 3;
        if rail != want {
            bad.push(format!("{path}: {h2} h2, rail {rail}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(pages.iter().any(|p| p.1.contains("class=\"outline rail\"")));
}

fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).expect("read dir") {
        let p = e.expect("entry").path();
        if p.is_dir() {
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// Every markdown parse in xtask takes the app's options, so a new render path cannot drift:
/// no bare parser, no empty or ad hoc option set.
#[test]
fn every_markdown_parse_uses_the_shared_options() {
    let mut files = Vec::new();
    rust_files(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let ext = ["Parser", "::new_ext("].concat();
    let bare = ["Parser", "::new("].concat();
    let ad_hoc = ["pulldown_cmark", "::Options"].concat();
    let ad_hoc_use = ["Options", "::"].concat();
    let shared = ["gitterrace_web", "::markdown_options()"].concat();
    let mut bad = Vec::new();
    for f in files {
        if f.file_name().is_some_and(|n| n == "tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&f).expect("read");
        for (n, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            let wrong = (line.contains(&ext) && !line.contains(&shared))
                || line.contains(&bare)
                || line.contains(&ad_hoc)
                || (line.contains(&ad_hoc_use)
                    && !line.contains("textwrap")
                    && !line.contains(&shared));
            if wrong {
                bad.push(format!("{}:{}: {t}", f.display(), n + 1));
            }
        }
    }
    assert!(bad.is_empty(), "markdown parsed without the shared options:\n{}", bad.join("\n"));
}

/// A docs page whose source has a table renders a table, never the raw delimiter row.
#[test]
fn docs_pages_with_a_table_render_a_table() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let delimiter = ["| ", "--- |"].concat();
    let (pages, _) = generated();
    let mut seen = 0;
    for (path, ..) in crate::docs_index::on_site() {
        let src = std::fs::read_to_string(root.join(path)).expect("source");
        let has_table = src.lines().any(|l| {
            let t = l.trim();
            t.starts_with('|') && t.contains("---") && t.chars().all(|c| "|-: ".contains(c))
        });
        if !has_table {
            continue;
        }
        seen += 1;
        let page = format!("{}index.html", docs::page_path(docs::stem(path)));
        let html = &pages.iter().find(|p| p.0 == page).expect("built page").1;
        assert!(html.contains("<table"), "{path}: no table on the site");
        assert!(!html.contains(&delimiter), "{path}: raw table row on the site");
    }
    assert!(seen > 0, "no docs page has a table, the guard checks nothing");
}

/// The class this guards: a generated page with a line over the width that a break could have
/// fixed, from any part of any page (a tag, a meta description, prose, a table, the footer).
/// Only a line holding an unbreakable token (a single URL) or inside `<pre>` may stay wider.
#[test]
fn no_generated_line_is_wider_than_the_width_where_it_could_break() {
    let (pages, _) = generated();
    let mut bad = Vec::new();
    for (path, html) in &pages {
        for line in super::fit::breakable_wide_lines(html) {
            bad.push(format!("{path}: {line}"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    let planted = format!("<p>{}</p>\n", "a wide line of prose ".repeat(8));
    assert_ne!(super::fit::breakable_wide_lines(&planted), [] as [std::string::String; 0]);
}

/// Guard class: every heading id of every published page is the id the app gives that heading,
/// so a `#fragment` written against the app or the repository host lands on the site too. Red
/// for any heading where the two slug rules part, such as punctuation inside a word.
#[test]
fn every_published_heading_id_matches_the_app() {
    let root = crate::repo_root();
    let mut bad = Vec::new();
    for (rel, ..) in crate::docs_index::on_site() {
        let text = std::fs::read_to_string(root.join(rel)).expect("doc");
        let app = crate::docs_index::anchors(&text);
        for id in docs::anchors(&text) {
            if !app.contains(&id) {
                bad.push(format!("{rel}: #{id} is not an id the app renders"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// The rule itself, on the shapes that once parted: punctuation inside a word is dropped, not
/// a word break.
#[test]
fn the_site_slug_keeps_the_app_rule() {
    assert_eq!(docs::slug("HTTP (/api/v1)"), "http-apiv1");
    assert_eq!(docs::slug("Code age and releases"), "code-age-and-releases");
}

/// Guard class: the docs nav, its group headings and the pager drift from the docs index, in
/// order, label or membership. Every built docs page's nav must list the index's groups under
/// their headings in index order, and its previous and next links must be its index neighbors.
#[test]
fn docs_nav_headings_order_and_pager_follow_the_index() {
    let (pages, _) = generated();
    let mut want: Vec<String> = Vec::new();
    let mut order: Vec<String> = Vec::new();
    let mut last = "";
    for (rel, heading, _) in crate::docs_index::on_site() {
        if heading != last {
            want.push(format!("# {heading}"));
            last = heading;
        }
        let p = docs::page_path(docs::stem(rel));
        want.push(p.clone());
        order.push(p);
    }
    for (i, p) in order.iter().enumerate() {
        let html = &pages
            .iter()
            .find(|(f, _)| *f == format!("{p}index.html"))
            .unwrap_or_else(|| panic!("{p}: not built"))
            .1;
        let nav = html
            .split_once("<nav class=\"docnav\"")
            .and_then(|(_, r)| r.split_once("</nav>"))
            .map(|(n, _)| n)
            .expect("docnav");
        let mut got = Vec::new();
        for part in nav.split('<').skip(1) {
            if let Some(h) = part.strip_prefix("h3>") {
                got.push(format!("# {h}"));
            } else if let Some(a) = part.strip_prefix("a href=\"/") {
                got.push(a.split('"').next().unwrap_or("").to_owned());
            }
        }
        assert_eq!(got, want, "{p}: nav differs from the index");
        let under = html.split_once("<nav class=\"pager\"").map(|(_, r)| r).expect("pager");
        let prev = i.checked_sub(1).map(|j| &order[j]);
        let next = order.get(i + 1);
        for (rel, link) in [("prev", prev), ("next", next)] {
            let has = |q: &str| under.contains(&format!("href=\"/{q}\" rel=\"{rel}\""));
            match link {
                Some(q) => assert!(has(q), "{p}: {rel} is not {q}"),
                None => assert!(!under.contains(&format!("rel=\"{rel}\"")), "{p}: {rel}"),
            }
        }
    }
}

/// The site carries the app's syntax colours, scoped to `pre`, through the tokens' variables.
#[test]
fn site_css_carries_the_app_syntax_colours() {
    let css = super::css(
        "/* BEGIN generated */\n:root{}\n/* END generated */\n/* roles */\n",
        include_str!("../../../crates/gitterrace-web/static/syntax.css"),
    )
    .expect("css");
    assert!(css.contains("pre [class~=s-keyword]"), "{css}");
    assert!(css.contains("var(--syn-keyword)"), "{css}");
    assert!(!css.contains("--md-k"), "{css}");
}

/// The class this guards: an asset URL whose `?v=` is not the hash of the bytes served under it,
/// so a year-long cache could keep a stale stylesheet or font. Every `/assets/x?v=h` on every
/// page and stylesheet must name the sha256 of the built `assets/x` as written.
#[test]
fn every_asset_version_is_the_hash_of_the_file_served() {
    let root = crate::repo_root();
    let ws = crate::facts::load(&root).expect("facts");
    let outs = render(&root, &ws).expect("render");
    // The hash of each file as built, the same digest the versioning names.
    let built = super::sums(&outs);
    let texts: Vec<(String, String)> = outs
        .iter()
        .filter_map(|o| match o {
            Out::File { path, body } => Some((path.clone(), body.clone())),
            _ => None,
        })
        .collect();
    let mut bad = Vec::new();
    let mut seen = 0;
    for (path, text) in &texts {
        for (at, _) in text.match_indices("/assets/") {
            let rest = &text[at + "/assets/".len()..];
            let Some((file, tail)) = rest.split_once("?v=") else {
                continue;
            };
            if file.contains(['"', ')', ' ', '<']) {
                continue;
            }
            let v: String = tail.chars().take_while(char::is_ascii_hexdigit).collect();
            seen += 1;
            if built.get(file) != Some(&v) {
                bad.push(format!("{path}: /assets/{file}?v={v}, built {:?}", built.get(file)));
            }
        }
    }
    assert!(seen > 10, "the site names its assets");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
