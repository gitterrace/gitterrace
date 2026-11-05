//! A form's submit button is the one shared button, so every form's action looks the same. A
//! rule that styles `button` under a form or other container class (`.cmp button`,
//! `.searchform button`) gives one page its own look; only the shared rules and controls that
//! are not submit buttons (segmented control, menus, the repo switcher) may style a button.

use std::path::Path;

/// Selector parts allowed to style a button: the shared rule and controls that are not a
/// form's submit button.
const ALLOWED: [&str; 4] = [".segs", "button.branch", "button.reponame", "button:"];

/// Selectors in `css` that restyle a button through a container class.
fn scoped_buttons(css: &str) -> Vec<String> {
    let mut bad = Vec::new();
    for rule in css.split('}') {
        let Some((sel, _)) = rule.split_once('{') else {
            continue;
        };
        for s in sel.split(',').map(str::trim) {
            let words: Vec<&str> = s
                .split([' ', '>', '+', '~'])
                .filter(|w| !w.is_empty())
                .collect();
            let scoped = words.len() > 1 && words.last().is_some_and(|w| w.starts_with("button"));
            if scoped && !ALLOWED.iter().any(|a| s.contains(a)) {
                bad.push(s.to_owned());
            }
        }
    }
    bad
}

#[test]
fn no_page_restyles_the_shared_button() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
    let mut bad = Vec::new();
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "css") {
            let css = std::fs::read_to_string(&p).unwrap_or_default();
            bad.extend(
                scoped_buttons(&css)
                    .into_iter()
                    .map(|s| format!("{}: {s}", p.display())),
            );
        }
    }
    assert!(bad.is_empty(), "a page styles its own button: {bad:?}");
}

#[test]
fn the_check_sees_a_scoped_button() {
    assert_eq!(
        scoped_buttons(".cmp button{ background: red }"),
        [".cmp button"]
    );
    assert_eq!(
        scoped_buttons("form.x>button.go{ x: y }"),
        ["form.x>button.go"]
    );
    let shared = "button{ x: y }button:hover{ x: y }.segs button{ x: y }";
    assert!(scoped_buttons(shared).is_empty());
}
