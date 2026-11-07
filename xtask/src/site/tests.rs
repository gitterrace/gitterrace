use super::shots::{SHOT_DIR, SITE_SHOT_DIR};
use super::*;

/// One figure per shot, a light/dark pair inside it, and a lightbox that is labelled, takes
/// focus and does not announce its image twice: per shot only the tile's light image has
/// an alt, every dark copy and both lightbox images are decorative.
#[test]
fn gallery_has_one_figure_per_shot_and_an_accessible_lightbox() {
    let g = gallery();
    assert!(!g.contains("<figure><figure"), "nested figure");
    assert_eq!(g.matches("<figure>").count(), SHOTS.len());
    assert_eq!(g.matches("<span class=\"shot\">").count(), SHOTS.len() * 2);
    assert_eq!(g.matches("class=\"on-dark\"").count(), SHOTS.len() * 2);
    assert_eq!(g.matches("alt=\"\"").count(), SHOTS.len() * 3);
    assert_eq!(g.matches("tabindex=\"-1\"").count(), SHOTS.len());
    assert_eq!(g.matches("aria-label=\"Close ").count(), SHOTS.len());
}

/// A tile loads only its thumbnail, and the full-size file is left to the lightbox.
#[test]
fn tiles_never_reference_the_full_size_screenshot() {
    for (name, alt, cap) in SHOTS {
        let h = shot_html(name, alt, cap);
        let (tile, lightbox) = h.split_once("<a class=\"lightbox\"").expect("lightbox");
        for c in ["light", "dark"] {
            assert!(!tile.contains(&format!("screenshots/{name}-{c}.webp")));
            assert!(!h.contains(".png"), "shot {name} links a PNG");
            assert!(tile.contains(&format!("screenshots/{name}-{c}.thumb.webp")));
            assert!(
                !lightbox.contains(".thumb."),
                "lightbox {name} loads a thumbnail"
            );
        }
        assert!(lightbox.contains(&format!("screenshots/{name}-dark.webp")));
        assert!(tile.contains("width=\"1280\" height=\"800\" loading=\"lazy\""));
    }
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

/// A PNG header of the given size, enough for `shot_size`.
fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.push(0);
    b
}

#[test]
fn readme_shots_are_the_linked_pngs() {
    let r = "<img src=\"docs/screenshots/b-light.png\" alt=\"x\">\n\
                 srcset=\"docs/screenshots/a-dark.png\" docs/screenshots/b-light.png \
                 docs/screenshots/x.svg";
    assert_eq!(readme_shots(r), ["a-dark.png", "b-light.png"]);
}

#[test]
fn a_missing_unreferenced_or_wrongly_sized_screenshot_fails() {
    let dir = std::env::temp_dir().join(format!("gt-shots-{}", std::process::id()));
    let shots = dir.join(SHOT_DIR);
    let site = dir.join(SITE_SHOT_DIR);
    std::fs::create_dir_all(&shots).expect("mkdir");
    std::fs::create_dir_all(&site).expect("mkdir");
    std::fs::write(
        dir.join("README.md"),
        "<source srcset=\"docs/screenshots/overview-dark.png\">\n<img \
            src=\"docs/screenshots/overview-light.png\">\n",
    )
    .expect("readme");
    let err = check_shots(&dir)
        .expect_err("empty folder fails")
        .to_string();
    assert!(
        err.contains("overview-dark.png is linked by README.md"),
        "{err}"
    );
    assert!(err.contains("make site-shots"), "{err}");
    for c in ["light", "dark"] {
        std::fs::write(shots.join(format!("overview-{c}.png")), png(2560, 1600)).expect("write");
    }
    // A PNG the README does not link fails, though the site has a shot of that name.
    std::fs::write(shots.join("blame-dark.png"), png(2560, 1600)).expect("write");
    let err = check_shots(&dir)
        .expect_err("unlinked png fails")
        .to_string();
    assert!(
        err.contains("blame-dark.png is not referenced by README.md"),
        "{err}"
    );
    assert!(err.contains("make site-shots"), "{err}");
    std::fs::remove_file(shots.join("blame-dark.png")).expect("rm");
    std::fs::write(shots.join("overview-dark.png"), png(1280, 800)).expect("write");
    let err = check_shots(&dir).expect_err("small png fails").to_string();
    assert!(err.contains("overview-dark.png is 1280x800"), "{err}");
    std::fs::write(shots.join("overview-dark.png"), png(2560, 1600)).expect("write");
    let err = check_shots(&dir).expect_err("no webp fails").to_string();
    assert!(err.contains("is missing (make site-shots)"), "{err}");
    for (n, ..) in SHOTS {
        for c in ["light", "dark"] {
            std::fs::write(site.join(format!("{n}-{c}.webp")), webp(2560, 1600)).expect("write");
            std::fs::write(site.join(format!("{n}-{c}.thumb.webp")), webp(1280, 800))
                .expect("write");
        }
    }
    check_shots(&dir).expect("complete set passes");
    std::fs::write(site.join("blame-dark.thumb.webp"), webp(1200, 750)).expect("write");
    let err = check_shots(&dir).expect_err("wrong size fails").to_string();
    assert!(err.contains("blame-dark.thumb.webp is 1200x750"), "{err}");
    assert!(err.contains("make site-shots"), "{err}");
    std::fs::write(site.join("blame-dark.thumb.webp"), webp(1280, 800)).expect("write");
    std::fs::write(site.join("blame-dark.png"), b"x").expect("write");
    let err = check_shots(&dir).expect_err("site png fails").to_string();
    assert!(err.contains("not referenced"), "{err}");
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

/// Every heading of an internals doc is an id on its page, so a link can name the rule.
#[test]
fn internals_pages_give_every_heading_its_slug_as_id() {
    let text = "# Title\n\n## Code age and releases\n\ntext\n";
    assert_eq!(docs::anchors(text), ["title", "code-age-and-releases"]);
    let links = docs::Links::new("https://r");
    let html = docs::body(text, "docs/internals/x.md", &links, &mut |_, h| {
        docs::slug(h)
    });
    assert!(html.contains("<h2 id=\"code-age-and-releases\">Code age and releases</h2>"));
    assert_eq!(docs::page_path("architecture"), "docs/architecture/");
}

/// The install tabs show every way, with the first open, and each way has a radio, a label
/// and a panel the CSS can reach.
#[test]
fn install_tabs_pair_every_radio_with_a_panel() {
    for way in ["curl", "brew", "cargo"] {
        assert!(PAGE_CSS.contains(&format!("#install-{way}:checked~#panel-{way}")));
    }
}

/// Every `attr="value"` of `html`, in order.
pub(super) fn attrs<'a>(html: &'a str, attr: &str) -> Vec<&'a str> {
    let pat = format!(" {attr}=\"");
    html.split(&pat)
        .skip(1)
        .filter_map(|r| r.split_once('"').map(|v| v.0))
        .collect()
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
                let rel = path.strip_prefix("site/").unwrap_or(&path).to_owned();
                if std::path::Path::new(&rel)
                    .extension()
                    .is_some_and(|e| e == "html")
                {
                    pages.push((rel.clone(), body));
                }
                files.push(rel);
            }
            Out::Bytes { path, .. } => {
                files.push(path.strip_prefix("site/").unwrap_or(&path).to_owned());
            }
            Out::Block { .. } => {}
        }
    }
    // The screenshots are made by `make site-shots` and checked on disk. The touch icon is
    // ignored by git and rendered by `make site`, so it is known by name.
    files.push("assets/apple-touch-icon.png".to_owned());
    for dir in ["assets/screenshots"] {
        let entries = std::fs::read_dir(root.join("site").join(dir)).expect("site dir");
        for e in entries.flatten() {
            files.push(format!("{dir}/{}", e.file_name().to_string_lossy()));
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
    let err = docs::check_unique_ids("docs/x/", html)
        .expect_err("duplicate")
        .to_string();
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
    assert!(
        bad.contains("/docs/nope/#x: no generated file docs/nope/index.html"),
        "{bad}"
    );
    assert!(
        bad.contains("#nope: no id nope in docs/why/index.html"),
        "{bad}"
    );
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
