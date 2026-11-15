//! Images, audio, video and PDFs: what the blob view and commit diffs show for them, and the
//! content types `/raw` serves. Pure functions over a path and bytes, so they are testable.

use crate::fmt::{enc_path, enc_seg, esc, href, size};

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
pub fn plain_media(path: &str) -> bool {
    matches!(kind(path), Some(Kind::Image | Kind::Audio | Kind::Video))
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
fn dimensions(path: &str, b: &[u8]) -> Option<(u32, u32)> {
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
    href(&format!("/raw/{}/{}", enc_seg(rev), enc_path(path)))
}

/// Where one side of an image diff loads from: a committed blob, or the working copy.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Side<'a> {
    At(&'a str, &'a str),
    /// The file on disk, through the local-only `/worktree/raw` route.
    Worktree(&'a str),
}

impl<'a> Side<'a> {
    fn url(self) -> String {
        match self {
            Self::At(rev, path) => raw_url(rev, path),
            Self::Worktree(path) => href(&format!("/worktree/raw/{}", enc_path(path))),
        }
    }

    const fn file(self) -> &'a str {
        match self {
            Self::At(_, path) | Self::Worktree(path) => path,
        }
    }
}

/// One `<img>` (or media element) at `url`. A known pixel size reserves the box before load.
fn element_at(k: Kind, url: &str, path: &str, dims: Option<(u32, u32)>) -> String {
    let name = esc(path.rsplit('/').next().unwrap_or(path));
    match k {
        Kind::Image | Kind::Svg => {
            let size = dims.map_or_else(String::new, |(w, h)| {
                format!(r#" width="{w}" height="{h}""#)
            });
            format!(r#"<img class="media-img" src="{url}" alt="{name}"{size} loading="lazy">"#)
        }
        // No embedded viewer: a crafted PDF would run in the app's origin.
        Kind::Pdf => format!(r#"<a class="media-file" href="{url}" download>Download {name}</a>"#),
        Kind::Audio => {
            format!(r#"<audio class="media-av" src="{url}" controls preload="metadata"></audio>"#)
        }
        Kind::Video => {
            format!(r#"<video class="media-av" src="{url}" controls preload="metadata"></video>"#)
        }
    }
}

fn source_details(src: &str) -> String {
    format!(
        "<details class=\"media-src\"><summary>Code</summary>\
            <pre class=\"body\">{}</pre></details>",
        esc(src)
    )
}

/// The blob view body for a media file: the preview on a checkerboard, a facts line and links.
pub(crate) fn blob_html(
    k: Kind,
    len: u64,
    side: Side<'_>,
    bytes: &[u8],
    svg_src: Option<&str>,
) -> String {
    let path = side.file();
    let url = side.url();
    let px = dimensions(path, bytes);
    let dims = px.map_or_else(String::new, |(w, h)| format!("{w} x {h} · "));
    // Without `/raw` there is no byte to show or download, only the facts.
    if !crate::features::on(gitterrace_config::Feature::Raw) {
        let mut out = format!(
            r#"<div class="media"><p class="media-facts muted">{dims}{}</p></div>"#,
            size(len)
        );
        if let Some(src) = svg_src {
            out.push_str(&source_details(src));
        }
        return out;
    }
    let mut out = format!(
        "<div class=\"media\"><div class=\"media-stage{}\">{}</div>\
            <p class=\"media-facts muted\">{dims}{} · <a href=\"{url}\">Raw</a> · \
            <a href=\"{url}\" download>Download</a></p></div>",
        if matches!(k, Kind::Image | Kind::Svg) {
            " checker"
        } else {
            ""
        },
        element_at(k, &url, path, px),
        size(len),
    );
    if let Some(src) = svg_src {
        out.push_str(&source_details(src));
    }
    out
}

/// What one side of a changed image weighs: its size in bytes and its pixel size when the
/// header says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) len: u64,
    pub(crate) dims: Option<(u32, u32)>,
}

impl Facts {
    pub(crate) fn of(path: &str, bytes: &[u8]) -> Self {
        Self {
            len: bytes.len() as u64,
            dims: dimensions(path, bytes),
        }
    }
}

/// The header line of a changed image in place of `+0 -0`, which says nothing about a picture:
/// "1400 x 900, 213 KiB to 208 KiB", "new, 1400 x 900, 213 KiB" or "removed, 213 KiB". The
/// pixel size shows once when both sides agree, else as "w x h to w x h".
pub(crate) fn diff_counts(old: Option<Facts>, new: Option<Facts>) -> String {
    let px = |d: Option<(u32, u32)>| d.map(|(w, h)| format!("{w} x {h}"));
    let line = match (old, new) {
        (Some(o), Some(n)) => {
            let dims = match (px(o.dims), px(n.dims)) {
                (Some(a), Some(b)) if a == b => format!("{a}, "),
                (Some(a), Some(b)) => format!("{a} to {b}, "),
                (_, Some(b)) => format!("{b}, "),
                _ => String::new(),
            };
            format!("{dims}{} to {}", size(o.len), size(n.len))
        }
        (None, Some(n)) => px(n.dims).map_or_else(
            || format!("new, {}", size(n.len)),
            |d| format!("new, {d}, {}", size(n.len)),
        ),
        (Some(o), None) => format!("removed, {}", size(o.len)),
        (None, None) => return String::new(),
    };
    format!(
        "<span class=\"counts media-counts muted\">{}</span>",
        esc(&line)
    )
}

/// One side of an image diff: where it loads from and its pixel size when known.
pub(crate) type Pic<'a> = (Side<'a>, Option<(u32, u32)>);

/// Before and after of one changed image, side by side; one side for added or deleted files.
pub(crate) fn diff_html(old: Option<Pic<'_>>, new: Option<Pic<'_>>) -> String {
    let img = |(at, dims): Pic<'_>| {
        let path = at.file();
        element_at(kind(path).unwrap_or(Kind::Image), &at.url(), path, dims)
    };
    let side = |label: &str, at: Option<Pic<'_>>| {
        at.map_or_else(String::new, |at| {
            format!(
                "<figure class=\"media-side\">\
                    <figcaption class=\"muted\">{label}</figcaption>\
                    <div class=\"media-stage checker\">{}</div></figure>",
                img(at)
            )
        })
    };
    let (Some(o), Some(n)) = (old, new) else {
        return format!(
            r#"<div class="media-diff">{}{}</div>"#,
            side("Before", old),
            side("After", new)
        );
    };
    // Both sides: side by side, or one over the other with a slider, where a small change
    // between two near-identical pictures is easier to find.
    let view = crate::seg::Seg {
        name: "Image view".to_owned(),
        panes: format!("view-{}", n.0.file()),
        choices: vec![
            crate::seg::Choice::new("Side by side", "sides", true),
            crate::seg::Choice::new("Overlay", "overlay", false),
        ],
        ..crate::seg::Seg::default()
    };
    format!(
        concat!(
            r#"<div class="media-view paned" data-media-view>"#,
            r#"<div class="media-bar">{view}</div>"#,
            r#"<div class="media-diff" data-pane="sides">{}{}</div>"#,
            r#"<div class="media-overlay" data-pane="overlay">"#,
            r#"<div class="media-stage checker"><div class="media-stack">"#,
            r#"<div class="media-before">{}</div><div class="media-after">{}</div>"#,
            r#"</div></div><label class="media-cut muted">Before"#,
            r#"<input type="range" min="0" max="100" value="50""#,
            r#" aria-label="Before and after split" data-media-cut>After</label>"#,
            r#"</div></div>"#,
        ),
        side("Before", old),
        side("After", new),
        img(o),
        img(n),
        view = view,
    )
}

#[cfg(test)]
mod tests {
    use super::{Facts, Kind, Side, content_type, diff_counts, diff_html, dimensions, kind};

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
    fn content_types_by_extension() {
        assert_eq!(content_type("a.PNG"), "image/png");
        assert_eq!(content_type("a.rs"), "text/plain; charset=utf-8");
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
        let both = diff_html(
            Some((Side::At("p", "a.png"), None)),
            Some((Side::At("c", "a.png"), Some((7, 5)))),
        );
        // The known size reserves the picture's box before it loads.
        assert!(both.contains(r#"width="7" height="5" loading"#), "{both}");
        assert!(both.contains(r#"alt="a.png" loading"#), "{both}");
        // Side by side and the overlay each hold both pictures.
        assert_eq!(both.matches("<img").count(), 4);
        assert!(both.contains("/raw/p/a.png") && both.contains("/raw/c/a.png"));
        assert_eq!(
            diff_html(None, Some((Side::At("c", "a.png"), None)))
                .matches("<img")
                .count(),
            1
        );
    }

    #[test]
    fn a_changed_image_header_says_pixels_and_bytes() {
        let f = |len, dims| Some(Facts { len, dims });
        let both = diff_counts(f(2048, Some((1400, 900))), f(1024, Some((1400, 900))));
        assert!(both.contains(">1400 x 900, 2.0 KiB to 1.0 KiB<"), "{both}");
        let resized = diff_counts(f(10, Some((10, 10))), f(20, Some((20, 20))));
        assert!(resized.contains(">10 x 10 to 20 x 20, "), "{resized}");
        assert!(diff_counts(None, f(10, Some((4, 2)))).contains(">new, 4 x 2, "));
        assert!(diff_counts(f(10, None), None).contains(">removed, "));
        assert!(!both.contains("+0"));
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\x05\x78\0\0\x03\x84";
        assert_eq!(Facts::of("a.png", png).dims, Some((1400, 900)));
    }

    #[test]
    fn a_modified_image_offers_side_by_side_and_overlay() {
        let both = diff_html(
            Some((Side::At("p", "a.png"), None)),
            Some((Side::At("c", "a.png"), None)),
        );
        assert!(both.contains("aria-label=\"Image view\""), "{both}");
        assert!(both.contains("data-show=\"overlay\""), "{both}");
        assert!(both.contains("value=\"sides\" checked"), "{both}");
        assert!(both.contains("data-media-cut"), "{both}");
        let added = diff_html(None, Some((Side::At("c", "a.png"), None)));
        assert!(
            !added.contains("data-show"),
            "one side has nothing to compare: {added}"
        );
    }
}
