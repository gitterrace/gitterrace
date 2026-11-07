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
                out.push_str(&base64(bytes));
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

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{base64, drop_font_preloads, inline_fonts};

    #[test]
    fn base64_matches_known_vectors() {
        // RFC 4648 section 10 vectors, as bytes like token.rs writes them.
        let cases: [(&[u8], &str); 5] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
        ];
        for (raw, want) in cases {
            assert_eq!(base64(raw), want);
        }
    }

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
