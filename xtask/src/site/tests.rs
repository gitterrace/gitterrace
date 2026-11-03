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
                 srcset=\"docs/screenshots/a-dark.png\" docs/screenshots/b-light.png docs/screenshots/x.svg";
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
            "<source srcset=\"docs/screenshots/overview-dark.png\">\n<img src=\"docs/screenshots/overview-light.png\">\n",
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
