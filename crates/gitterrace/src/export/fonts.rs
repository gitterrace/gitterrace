//! Fonts in a copy opened from disk. A browser refuses a font fetched from a `file://` URL
//! (each file is its own origin), so the copy carries each embedded font inside the
//! stylesheet as a `data:` URL and drops the font preloads, which would fetch and fail.

const FONT_URL: &str = "url(/static/fonts/";

/// `css` with every embedded font named by `url(/static/fonts/<name>)` written inline.
pub(super) fn inline_fonts(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(at) = rest.find(FONT_URL) {
        out.push_str(&rest[..at]);
        let tail = &rest[at + FONT_URL.len()..];
        let end = tail.find(')').unwrap_or(tail.len());
        match gitterrace_web::font_bytes(&tail[..end]) {
            Some(bytes) => {
                out.push_str("url(data:font/woff2;base64,");
                out.push_str(&gitterrace_actions::base64(
                    bytes,
                    gitterrace_actions::Base64::Standard,
                ));
            }
            None => out.push_str(&rest[at..at + FONT_URL.len() + end]),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// `html` without its `<link rel="preload" ... as="font">` lines.
pub(super) fn drop_font_preloads(html: &str) -> String {
    html.lines()
        .filter(|l| !(l.contains("rel=\"preload\"") && l.contains("as=\"font\"")))
        .collect::<Vec<_>>()
        .join("\n")
        + if html.ends_with('\n') { "\n" } else { "" }
}

#[cfg(test)]
mod tests {
    use super::{drop_font_preloads, inline_fonts};

    #[test]
    fn every_embedded_font_is_inlined_and_others_stay() {
        let css = inline_fonts(
            "@font-face{src:url(/static/fonts/fraunces.woff2) format(\"woff2\")}\
             @font-face{src:url(/static/fonts/none.woff2)}",
        );
        assert!(
            css.contains("url(data:font/woff2;base64,"),
            "{}",
            &css[..80]
        );
        assert!(!css.contains("fraunces.woff2"));
        assert!(css.ends_with("@font-face{src:url(/static/fonts/none.woff2)}"));
    }

    #[test]
    fn font_preloads_are_dropped() {
        let html = concat!(
            "<head>\n",
            "<link rel=\"preload\" href=\"/static/fonts/a.woff2\" as=\"font\" crossorigin>\n",
            "<link rel=\"stylesheet\" href=\"/static/app.css\">\n",
        );
        assert_eq!(
            drop_font_preloads(html),
            "<head>\n<link rel=\"stylesheet\" href=\"/static/app.css\">\n"
        );
    }
}
