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
        // The served stylesheet names `<font>.woff2?v=<version>`; the bytes go by name.
        let name = tail[..end].split('?').next().unwrap_or_default();
        match gitterrace_web::font_bytes(name) {
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

/// Whether `tag` carries the attribute `name` with `value`, quoted either way or not at all.
/// The preloads are found by what they mean, not by how the template spells them, so an
/// attribute added, dropped or reordered there cannot stop export from finding them.
fn has_attr(tag: &str, name: &str, value: &str) -> bool {
    tag.split_ascii_whitespace().any(|word| {
        word.trim_end_matches(['>', '/'])
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix('='))
            .is_some_and(|v| v.trim_matches(['"', '\'']).eq_ignore_ascii_case(value))
    })
}

/// `html` without its `<link rel="preload" ... as="font">` tags, each removed whole with the
/// line break after it: a tag the template wraps over two lines must not leave its second line
/// behind as text.
pub(super) fn drop_font_preloads(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<link") {
        let Some(len) = rest[at..].find('>').map(|e| e + 1) else {
            break;
        };
        let tag = &rest[at..at + len];
        out.push_str(&rest[..at]);
        if has_attr(tag, "rel", "preload") && has_attr(tag, "as", "font") {
            rest = &rest[at + len..];
            rest = rest.strip_prefix('\n').unwrap_or(rest);
        } else {
            out.push_str(tag);
            rest = &rest[at + len..];
        }
    }
    out.push_str(rest);
    out
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
        assert!(css.contains("url(data:font/woff2;base64,"), "{}", &css[..80]);
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

    /// A preload the template wraps over two lines goes whole: dropping it line by line left
    /// its second line as text on every exported page.
    #[test]
    fn a_wrapped_font_preload_leaves_no_text_behind() {
        let html = concat!(
            "<head>\n",
            "<link rel=\"preload\" href=\"/static/fonts/a.woff2\" as=\"font\"\n",
            " crossorigin>\n",
            "<link rel=\"stylesheet\" href=\"/static/app.css\">\n",
        );
        assert_eq!(
            drop_font_preloads(html),
            "<head>\n<link rel=\"stylesheet\" href=\"/static/app.css\">\n"
        );
        // The base template the pages are rendered from, as it is now.
        let base = include_str!("../../../gitterrace-web/templates/_base.html");
        assert!(base.contains("rel=\"preload\""), "the base template preloads no font");
        let out = drop_font_preloads(base);
        assert!(!out.contains("preload"), "a font preload stayed");
        assert!(!out.contains("crossorigin"), "a preload left text behind");
    }

    /// The tag is found by its meaning: a template that reorders the attributes, changes the
    /// quotes or adds one must not bring the preloads back into an export.
    #[test]
    fn a_font_preload_is_dropped_however_its_attributes_are_written() {
        for tag in [
            "<link as=\"font\" crossorigin rel=\"preload\" href=\"/static/fonts/a.woff2\">",
            "<link rel='preload' href='/static/fonts/a.woff2' as='font' crossorigin>",
            "<link rel=preload href=/static/fonts/a.woff2 as=font>",
            "<link rel=\"preload\" as=\"font\" type=\"font/woff2\" href=\"/a.woff2\" />",
            "<link\n rel=\"preload\"\n as=\"font\"\n href=\"/a.woff2\">",
        ] {
            assert_eq!(drop_font_preloads(&format!("<head>\n{tag}\n<p>")), "<head>\n<p>", "{tag}");
        }
    }

    #[test]
    fn links_that_only_resemble_a_font_preload_stay() {
        for tag in [
            "<link rel=\"preload\" href=\"/static/app.css\" as=\"style\">",
            "<link rel=\"preload\" href=\"/static/app.css\" data-as=\"font\" as=\"style\">",
            "<link rel=\"stylesheet\" href=\"/static/a.css?rel=preload&as=font\">",
        ] {
            assert_eq!(drop_font_preloads(tag), tag);
        }
    }
}
