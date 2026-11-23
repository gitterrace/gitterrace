use super::*;

const HEADER: &str = "<svg><g transform=\"translate(40 40) scale(3.2)\">\
    <path fill=\"#000000\" d=\"M0 0Z\"/></g>\
    <path fill=\"#101010\" d=\"M1 1H5V9Z\"/><path fill=\"#5b5e8d\" d=\"M2 2Q3 3 4 4Z\"/>\
    <path fill=\"#5b5e8d\" d=\"M9 9Z\"/></svg>";

#[test]
fn the_logo_keeps_the_mark_and_the_wordmark_and_drops_the_tagline() {
    let re = Re::new().expect("patterns");
    let (logo, square) = build(&re, HEADER).expect("build");
    assert!(logo.contains("M1 1H5V9Z") && logo.contains("M2 2Q3 3 4 4Z"));
    assert!(!logo.contains("M9 9Z"), "the tagline stays out");
    assert!(square.contains("viewBox=\"0 0 32 32\"") && !square.contains("translate(40 40)"));
}

/// The class this guards: a wordmark whose halves read as one color.
#[test]
fn halves_of_one_color_are_refused() {
    let re = Re::new().expect("patterns");
    let same = HEADER.replace("#101010", "#5b5e8d");
    assert!(build(&re, &same).is_err());
}

#[test]
fn a_wordmark_token_that_differs_from_the_header_fill_is_drift() {
    let re = Re::new().expect("patterns");
    let ok = "--wm-git:light-dark(#101010,#e3ecf6);--wm-terrace:var(--accent);--accent:#5b5e8d;";
    assert_eq!(token_drift(&re, HEADER, "light", ok).expect("drift"), Vec::<String>::new());
    let bad = ok.replace("#101010", "#000000");
    assert_eq!(token_drift(&re, HEADER, "light", &bad).expect("drift").len(), 1);
    assert_eq!(token_drift(&re, HEADER, "light", "").expect("drift").len(), 2);
}

#[test]
fn a_relative_path_command_is_refused_in_the_wordmark() {
    assert!(bbox("M1 1l2 2Z").is_err());
    assert_eq!(bbox("M1 2H5V9Z").expect("box"), (1.0, 2.0, 5.0, 9.0));
}

#[test]
fn relative_data_lands_on_the_same_points() {
    assert_eq!(relative("M1 2H5V9Z").expect("rel"), "m1 2h4v7z");
    assert_eq!(
        relative("M10.5 10Q10.25 9.5 10 9L9 9Z").expect("rel"),
        "m10.5 10q-.25-.5-.5-1l-1 0z"
    );
}

#[test]
fn wrapping_breaks_at_spaces_within_the_width() {
    let out = wrap(&"ab ".repeat(40), 30);
    assert!(out.lines().all(|l| l.len() <= 30));
    assert_eq!(out.replace('\n', " "), "ab ".repeat(40).trim_end().to_owned() + " ");
}

#[test]
fn numbers_are_short_and_stable() {
    assert_eq!(num(1.0), "1");
    assert_eq!(num(1.23456), "1.235");
    assert_eq!(num(0.0), "0");
}

/// The drift check: the tree holds what the headers generate.
#[test]
fn the_tree_matches_the_headers() {
    use crate::out::{Mode, apply};
    let root = crate::repo_root();
    let stale = apply(&root, outs(&root).expect("outs"), Mode::Check).expect("check");
    assert!(stale.is_empty(), "stale: {stale:?}");
}

#[test]
fn a_word_wider_than_the_line_is_cut_before_a_command_and_short_words_are_kept() {
    assert_eq!(split_long("m1.5-2l3q-1-1", 6), vec!["m1.5-2", "l3q-1", "-1"]);
    assert_eq!(split_long("abc", 6), vec!["abc"]);
    assert_eq!(split_long("m1.5-2l3q-1-1", 6).concat(), "m1.5-2l3q-1-1");
}

/// The class this guards: a wrap that cuts a token or lets whitespace be trimmed away.
#[test]
fn trim_comment_wrap_renders_to_the_input_within_the_width() {
    let one = "<svg class=\"logo\" viewBox=\"2 2 28 28\"><path class=\"w-git\" \
        d=\"M1 1 L2  2 L3 3 L4 4 L5 5 L6 6 L7 7 L8 8\"/> </svg>";
    let svg = one.repeat(9);
    let out = trim_wrap(&svg, 100, 58, 10);
    assert_eq!(out.replace("{#-\n-#}", ""), svg);
    for (i, line) in out.lines().enumerate() {
        let extra = if i == 0 { 58 } else { 0 } + if line.ends_with("</svg>") { 10 } else { 0 };
        assert!(line.chars().count() + extra <= 100, "{line}");
    }
    assert!(out.contains("{#-\n-#}"));
    for whole in ["class=\"w-git\"", "<svg ", "viewBox=", "class=\"logo\""] {
        assert_eq!(out.matches(whole).count(), 9, "{whole} is never split");
    }
}
