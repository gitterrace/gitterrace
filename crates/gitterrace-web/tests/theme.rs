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

/// The generated Rust palette and the stylesheet carry the same accent.
#[test]
fn palette_accent_is_in_tokens_css() {
    let css = include_str!("../static/tokens.css");
    for p in [gitterrace_web::palette::LIGHT, gitterrace_web::palette::DARK] {
        assert!(css.contains(&format!("--accent:{};", p.accent)));
    }
}
