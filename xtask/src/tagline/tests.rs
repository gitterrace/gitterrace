use super::*;

const SVG: &str = "<svg><path fill=\"#112233\" d=\"M0 0\"/>\
                   <path fill=\"#445566\" data-text=\"old\" \
                   data-sha=\"0000000000000000000000000000000000000000000000000000000000000000\" \
                   d=\"M1 1\"/></svg>";

fn drawn(d: &str, text: &str) -> String {
    format!(
        "<svg><path fill=\"#445566\" data-text=\"{}\" data-sha=\"{}\" d=\"{d}\"/></svg>",
        escape(text),
        sha(d)
    )
}

#[test]
fn a_header_that_spells_the_tagline_verifies() {
    assert!(verify(&drawn("M1 1Z", TAGLINE)).is_ok());
}

/// The class this guards: outlined words that say something other than the text.
#[test]
fn other_words_are_refused() {
    assert!(verify(&drawn("M1 1Z", "another sentence")).is_err());
}

/// The class this guards: a path edited by hand, or a stale digest.
#[test]
fn a_path_that_does_not_match_its_digest_is_refused() {
    let svg = drawn("M1 1Z", TAGLINE).replace("M1 1Z", "M2 2Z");
    assert!(verify(&svg).is_err());
    assert!(verify(SVG).is_err());
}

#[test]
fn the_committed_headers_verify() {
    let root = crate::repo_root();
    for theme in ["light", "dark"] {
        let svg =
            std::fs::read_to_string(root.join(format!("docs/brand/readme-header-{theme}.svg")))
                .expect("header");
        verify(&svg).expect("verify");
    }
}

#[test]
fn redraw_replaces_only_the_last_path_and_keeps_its_color() {
    let root = crate::repo_root();
    let out = redraw(&root, SVG).expect("redraw");
    assert!(out.starts_with("<svg><path fill=\"#112233\" d=\"M0 0\"/><path fill=\"#445566\""));
    verify(&out).expect("verify");
    assert_eq!(redraw(&root, &out).expect("redraw"), out, "idempotent");
}

/// The redraw is the committed outline up to the last digit: the same commands in the same order.
#[test]
fn redraw_keeps_the_committed_shape() {
    let root = crate::repo_root();
    let svg =
        std::fs::read_to_string(root.join("docs/brand/readme-header-light.svg")).expect("header");
    let out = redraw(&root, &svg).expect("redraw");
    let kinds = |s: &str| {
        let d = last_path(s).expect("path")[4].to_owned();
        d.chars()
            .filter(char::is_ascii_uppercase)
            .collect::<String>()
    };
    assert_eq!(kinds(&out), kinds(&svg));
}

#[test]
fn another_description_is_refused() {
    assert!(same_as(TAGLINE).is_ok());
    assert!(same_as("something else").is_err());
}

#[test]
fn numbers_lose_trailing_zeros() {
    assert_eq!(ntos(150.0), "150");
    assert_eq!(ntos(135.48), "135.48");
    assert_eq!(ntos(1.5), "1.5");
}
