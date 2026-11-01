//! Colours live in `static/tokens.css` only, and it is generated from the snapshot.

/// Any stylesheet besides tokens.css must reach colors through a token, so a new
/// file added here is checked the same way.
#[test]
fn stylesheets_hold_no_raw_colors() {
    let sheets = [
        ("app.css", include_str!("../static/app.css")),
        ("markdown.css", include_str!("../static/markdown.css")),
    ];
    for (name, css) in sheets {
        for (i, line) in css.lines().enumerate() {
            let raw = line.contains("rgb(") || line.contains("rgba(") || {
                line.match_indices('#').any(|(at, _)| {
                    let hex = line[at + 1..]
                        .chars()
                        .take_while(char::is_ascii_hexdigit)
                        .count();
                    matches!(hex, 3 | 4 | 6 | 8)
                })
            };
            assert!(!raw, "{name}:{} has a raw color, use a token", i + 1);
        }
    }
}

/// A rule that clips its overflow needs room for descenders: with a line height of 1,
/// the bottom of g, j, p, q and y is cut off.
#[test]
fn clipped_text_keeps_room_for_descenders() {
    for (name, css) in [
        ("app.css", include_str!("../static/app.css")),
        ("markdown.css", include_str!("../static/markdown.css")),
    ] {
        for rule in css.split('}') {
            let tight = rule.contains("/1 ") || rule.contains("line-height:1;");
            assert!(
                !(tight && rule.contains("overflow:hidden")),
                "{name}: `{}` clips overflow at line height 1, raise it",
                rule.trim().split('{').next().unwrap_or_default()
            );
        }
    }
}

/// `all:unset` loses to the global `button:hover` on hover, so a reset that
/// stops at the base state paints a button background on a text control.
#[test]
fn unset_resets_cover_hover() {
    let css = include_str!("../static/app.css");
    for rule in css.split('}') {
        let Some((sel, body)) = rule.split_once('{') else { continue };
        if !body.contains("all:unset") {
            continue;
        }
        let sels: Vec<&str> = sel.split(',').map(str::trim).collect();
        for base in sels.iter().filter(|s| !s.contains(':')) {
            assert!(
                sels.contains(&format!("{base}:hover").as_str()),
                "{base} resets with all:unset but not on :hover, so button:hover leaks through"
            );
        }
    }
}
