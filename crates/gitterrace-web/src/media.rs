//! Images, audio, video and PDFs: what the blob view and commit diffs show for them, and the
//! content types `/raw` serves. Pure functions over a path and bytes, so they are testable.

use crate::fmt::{enc, esc, href, size};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Image,
    Svg,
    Pdf,
    Audio,
    Video,
}

fn ext(path: &str) -> String {
    path.rsplit_once('.')
        .map_or_else(String::new, |(_, e)| e.to_ascii_lowercase())
}

/// What kind of media `path` is, by extension.
pub(crate) fn kind(path: &str) -> Option<Kind> {
    Some(match ext(path).as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" | "bmp" | "ico" => Kind::Image,
        "svg" => Kind::Svg,
        "pdf" => Kind::Pdf,
        "mp3" | "m4a" | "ogg" | "oga" | "wav" => Kind::Audio,
        "mp4" | "webm" | "mov" | "m4v" => Kind::Video,
        _ => return None,
    })
}

pub(crate) fn is_image(path: &str) -> bool {
    matches!(kind(path), Some(Kind::Image | Kind::Svg))
}

/// `Content-Type` for `/raw`.
pub(crate) fn content_type(path: &str) -> &'static str {
    match ext(path).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "wav" => "audio/wav",
        _ => "text/plain; charset=utf-8",
    }
}

/// Whether `/raw` may drop the sandbox. Only formats the browser renders without running
/// author code: scripts live in SVG and HTML, so those keep the sandbox.
pub(crate) fn plain_media(path: &str) -> bool {
    matches!(
        kind(path),
        Some(Kind::Image | Kind::Pdf | Kind::Audio | Kind::Video)
    )
}

fn be16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_be_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
    ])))
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn le16(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from(u16::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
    ])))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

fn jpeg(b: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < b.len() {
        if b[i] != 0xFF {
            i += 1;
            continue;
        }
        let m = b[i + 1];
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            return Some((be16(b, i + 7)?, be16(b, i + 5)?));
        }
        i += 2 + usize::try_from(be16(b, i + 2)?).ok()?;
    }
    None
}

fn webp(b: &[u8]) -> Option<(u32, u32)> {
    match b.get(12..16)? {
        b"VP8 " => Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF)),
        b"VP8L" => {
            let v = le32(b, 21)?;
            Some(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => {
            let w =
                u32::from(*b.get(24)?) | u32::from(*b.get(25)?) << 8 | u32::from(*b.get(26)?) << 16;
            let h =
                u32::from(*b.get(27)?) | u32::from(*b.get(28)?) << 8 | u32::from(*b.get(29)?) << 16;
            Some((w + 1, h + 1))
        }
        _ => None,
    }
}

/// Pixel size read from the file header, for the formats where that is a few bytes.
pub(crate) fn dimensions(path: &str, b: &[u8]) -> Option<(u32, u32)> {
    match ext(path).as_str() {
        "png" if b.starts_with(b"\x89PNG") => Some((be32(b, 16)?, be32(b, 20)?)),
        "gif" if b.starts_with(b"GIF8") => Some((le16(b, 6)?, le16(b, 8)?)),
        "jpg" | "jpeg" if b.starts_with(&[0xFF, 0xD8]) => jpeg(b),
        "bmp" if b.starts_with(b"BM") => {
            Some((le32(b, 18)?, le32(b, 22)?.cast_signed().unsigned_abs()))
        }
        "webp" if b.starts_with(b"RIFF") => webp(b),
        _ => None,
    }
    .filter(|(w, h)| *w > 0 && *h > 0)
}

fn raw_url(rev: &str, path: &str) -> String {
    href(&format!("/raw/{}/{}", enc(rev, false), enc(path, true)))
}

/// One `<img>` (or media element) for `path` at `rev`.
fn element(k: Kind, rev: &str, path: &str) -> String {
    let url = raw_url(rev, path);
    let name = esc(path.rsplit('/').next().unwrap_or(path));
    match k {
        Kind::Image | Kind::Svg => {
            format!(r#"<img class="media-img" src="{url}" alt="{name}" loading="lazy">"#)
        }
        Kind::Pdf => format!(
            r#"<object class="media-pdf" data="{url}" type="application/pdf"><a href="{url}">Download {name}</a></object>"#
        ),
        Kind::Audio => {
            format!(r#"<audio class="media-av" src="{url}" controls preload="metadata"></audio>"#)
        }
        Kind::Video => {
            format!(r#"<video class="media-av" src="{url}" controls preload="metadata"></video>"#)
        }
    }
}

/// The blob view body for a media file: the preview on a checkerboard, a facts line and links.
pub(crate) fn blob_html(
    k: Kind,
    rev: &str,
    path: &str,
    bytes: &[u8],
    svg_src: Option<&str>,
) -> String {
    let url = raw_url(rev, path);
    let dims = dimensions(path, bytes).map_or_else(String::new, |(w, h)| format!("{w} x {h} · "));
    let mut out = format!(
        r#"<div class="media"><div class="media-stage{}">{}</div><p class="media-facts muted">{dims}{} · <a href="{url}">Raw</a> · <a href="{url}" download>Download</a></p></div>"#,
        if matches!(k, Kind::Image | Kind::Svg) {
            " checker"
        } else {
            ""
        },
        element(k, rev, path),
        size(bytes.len() as u64),
    );
    if let Some(src) = svg_src {
        crate::fmt::push(
            &mut out,
            &format!(
                r#"<details class="media-src"><summary>Code</summary><pre class="body">{}</pre></details>"#,
                esc(src)
            ),
        );
    }
    out
}

/// Before and after of one changed image, side by side; one side for added or deleted files.
pub(crate) fn diff_html(old: Option<(&str, &str)>, new: Option<(&str, &str)>) -> String {
    let side = |label: &str, at: Option<(&str, &str)>| {
        at.map_or_else(String::new, |(rev, path)| {
            let k = kind(path).unwrap_or(Kind::Image);
            format!(
                r#"<figure class="media-side"><figcaption class="muted">{label}</figcaption><div class="media-stage checker">{}</div></figure>"#,
                element(k, rev, path)
            )
        })
    };
    format!(
        r#"<div class="media-diff">{}{}</div>"#,
        side("Before", old),
        side("After", new)
    )
}

#[cfg(test)]
mod tests {
    use super::{Kind, content_type, diff_html, dimensions, kind};

    #[test]
    fn classifies_and_types() {
        assert_eq!(kind("a/b.PNG"), Some(Kind::Image));
        assert_eq!(kind("x.svg"), Some(Kind::Svg));
        assert_eq!(kind("x.mov"), Some(Kind::Video));
        assert_eq!(kind("x.rs"), None);
        for (p, t) in [
            ("a.pdf", "application/pdf"),
            ("a.mp3", "audio/mpeg"),
            ("a.m4a", "audio/mp4"),
            ("a.ogg", "audio/ogg"),
            ("a.wav", "audio/wav"),
            ("a.mp4", "video/mp4"),
            ("a.webm", "video/webm"),
            ("a.mov", "video/quicktime"),
        ] {
            assert_eq!(content_type(p), t);
        }
    }

    #[test]
    fn reads_png_and_gif_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend([0, 0, 1, 0, 0, 0, 0, 20]);
        assert_eq!(dimensions("a.png", &png), Some((256, 20)));
        let gif = b"GIF89a\x10\0\x08\0";
        assert_eq!(dimensions("a.gif", gif), Some((16, 8)));
        assert_eq!(dimensions("a.png", b"junk"), None);
    }

    #[test]
    fn diff_shows_one_or_two_sides() {
        let both = diff_html(Some(("p", "a.png")), Some(("c", "a.png")));
        assert_eq!(both.matches("<img").count(), 2);
        assert!(both.contains("/raw/p/a.png") && both.contains("/raw/c/a.png"));
        assert_eq!(
            diff_html(None, Some(("c", "a.png")))
                .matches("<img")
                .count(),
            1
        );
    }
}
