//! Colours live in `static/tokens.css` only, and it is generated from the snapshot.

/// Any stylesheet besides tokens.css must reach colors through a token, so a new
/// file added here is checked the same way.
#[test]
fn stylesheets_hold_no_raw_colors() {
    let sheets = [
        ("app.css", include_str!("../static/app.css")),
        ("markdown.css", include_str!("../static/markdown.css")),
        ("media.css", include_str!("../static/media.css")),
        ("send.css", include_str!("../static/send.css")),
        ("insights.css", include_str!("../static/insights.css")),
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
        ("media.css", include_str!("../static/media.css")),
        ("send.css", include_str!("../static/send.css")),
        ("insights.css", include_str!("../static/insights.css")),
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
        let Some((sel, body)) = rule.split_once('{') else {
            continue;
        };
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

/// Owner rule: text is never cut with an ellipsis, with one exception. A commit message is a
/// single line that ends in an ellipsis and carries its full text as a tip. Every commit
/// message uses the one `.subj` class, so that is the only selector allowed to truncate.
#[test]
fn only_commit_messages_truncate() {
    for (name, css) in [
        ("app.css", include_str!("../static/app.css")),
        ("markdown.css", include_str!("../static/markdown.css")),
        ("media.css", include_str!("../static/media.css")),
        ("send.css", include_str!("../static/send.css")),
        ("insights.css", include_str!("../static/insights.css")),
    ] {
        for rule in css.split('}') {
            let squeezed: String = rule.split_whitespace().collect();
            if squeezed.contains("text-overflow:ellipsis") {
                let sel = rule.rsplit_once('{').map(|(s, _)| s).unwrap_or_default();
                let sel = sel.rsplit("*/").next().unwrap_or_default().trim();
                // `.reporow .today` ends in the last commit's subject and carries the
                // full line in its tooltip, so it is a commit message line too.
                assert!(
                    sel == ".subj" || sel == ".reporow .today",
                    "{name}: `{sel}` truncates text with an ellipsis, only commit message lines may"
                );
            }
        }
    }
}

/// Text-field styling (a fixed height, a minimum width, padding, a field border) on a bare
/// `input` selector reaches checkboxes and radios too, so a Markdown task list draws 240 px wide
/// rounded boxes. Text-field rules name the inputs they style; toggles keep one shared rule.
#[test]
fn text_field_rules_skip_toggles() {
    let sheets = [
        ("app.css", include_str!("../static/app.css")),
        ("markdown.css", include_str!("../static/markdown.css")),
        ("media.css", include_str!("../static/media.css")),
        ("send.css", include_str!("../static/send.css")),
        ("insights.css", include_str!("../static/insights.css")),
    ];
    for (name, css) in sheets {
        for rule in css.split('}') {
            let Some((sel, body)) = rule.split_once('{') else {
                continue;
            };
            let field = ["min-width:", "height:", "padding:"]
                .iter()
                .any(|p| body.contains(p));
            for s in sel.split(',') {
                // The whole selector is `input` (or `input:focus` and the like): nothing names
                // which inputs, so toggles match too. A selector that picks inputs by place or
                // parent (`.form>input:first-child`) or rules toggles out with `:not` is fine.
                let s = s.trim();
                let bare = s == "input" || s.starts_with("input:") && !s.contains("not(");
                assert!(
                    !(field && bare),
                    "{name}: `{s}` gives text-field sizing to every input, checkboxes too"
                );
            }
        }
    }
}
