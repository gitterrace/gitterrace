use super::*;

/// The class this guards: a role pair under its minimum, for any token pair in the matrix.
#[test]
fn a_low_contrast_pair_is_reported() {
    let pale: Colours = pairs()
        .iter()
        .flat_map(|(f, b, _)| [f.clone(), b.clone()])
        .map(|n| (n, "#f0f0f0".to_owned()))
        .collect();
    let bad = contrast_problems(&pale, &pale).expect("problems");
    assert!(bad.iter().any(|b| b.starts_with("light: --n-6 on --n-1")), "{bad:?}");
}

#[test]
fn contrast_is_the_wcag_ratio() {
    let black_on_white = contrast("#000000", "#ffffff").expect("contrast");
    assert!((black_on_white - 21.0).abs() < 1e-9);
}

#[test]
fn a_shadow_keeps_its_shape_and_pairs_its_color() {
    assert_eq!(pair("0 8px rgba(1)", "0 8px rgba(2)"), "0 8px light-dark(rgba(1),rgba(2))");
    assert_eq!(pair("#fff", "#000"), "light-dark(#fff,#000)");
}

/// The drift check: the tree holds what the snapshot generates.
#[test]
fn tokens_css_matches_the_snapshot() {
    use crate::out::{Mode, apply};
    let root = crate::repo_root();
    let stale = apply(&root, vec![render(&root).expect("render")], Mode::Check).expect("check");
    assert!(stale.is_empty(), "stale: {stale:?}");
}

/// The class this guards: a token the kit adds that the generator drops, so a page reading it
/// gets nothing. The display step reaches tokens.css as the kit's own clamp, and one that does
/// not sit above the scale is refused.
#[test]
fn the_display_step_is_emitted_and_must_sit_above_the_scale() {
    let ty = |min: u64| {
        serde_json::json!({
            "scalePx": [11, 26],
            "display": {"minPx": min, "maxPx": 74, "fluid": "1.2rem + 3.6vw",
                        "leading": 1, "tracking": "-.02em"}
        })
    };
    assert_eq!(
        display(&ty(34)).expect("display"),
        "--text-display:clamp(2.125rem,1.2rem + 3.6vw,4.625rem);\n--leading-display:1;\
         --tracking-display:-.02em;"
    );
    let err = display(&ty(20)).expect_err("under the scale top");
    assert!(err.to_string().contains("scale top 26 < minPx 20"), "{err}");
}
