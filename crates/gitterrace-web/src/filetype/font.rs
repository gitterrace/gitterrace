//! ttf, otf, woff and woff2 blobs: the family, the styles and the glyph count, read with
//! skrifa, and a sample the browser draws from `/raw` (`static/font.js`). The bytes come from
//! the repository and may be hostile, so three bounds hold before anything is read: a file
//! over [`BYTES_CAP`] is not opened, a woff table or block that would unpack past
//! [`DECODED_CAP`] in total is refused before its buffer exists, and the names shown are
//! cut to [`NAME_LEN`] with control characters dropped. A font that does not read shows no
//! facts and no sample, only the format line every binary blob gets.

use std::borrow::Cow;
use std::error::Error;
use std::io::Read as _;

use askama::Template;
use skrifa::raw::TableProvider as _;
use skrifa::string::StringId;
use skrifa::{FontRef, MetadataProvider as _};

/// Largest file whose tables are read. Not configurable: a read bound on a file from the
/// repository, which may be hostile. A bigger font (a CJK face) shows the format line only.
pub(crate) const BYTES_CAP: usize = 4 << 20;

/// Bytes a woff file may unpack to, in total. A 4 MiB brotli stream could otherwise declare
/// 100 times that. Not configurable: a bound against a decompression bomb.
const DECODED_CAP: usize = 32 << 20;

/// Named styles and axes listed. Not configurable: a display bound.
const NAMES_CAP: usize = 32;

/// Characters of one name shown.
const NAME_LEN: usize = 100;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Info {
    pub(crate) family: String,
    /// Named instances of a variable font, else the one style the font names.
    pub(crate) styles: String,
    pub(crate) glyphs: u32,
    /// `wght 100 to 900` for a variable font, else empty.
    pub(crate) axes: String,
}

#[derive(Template)]
#[template(path = "font.html")]
struct FontMarkup<'a> {
    f: &'a Info,
    glyphs: String,
    /// Where `font.js` loads the sample from, `None` without `/raw`.
    src: Option<&'a str>,
}

/// Whether `path` names a font this module reads.
pub(crate) fn is_font(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, e)| {
        ["ttf", "otf", "woff", "woff2"]
            .iter()
            .any(|f| e.eq_ignore_ascii_case(f))
    })
}

/// The facts and sample of a font, `None` when it does not read. A panic in a parser on a
/// crafted file is caught here, so one bad blob cannot take its request down.
pub(crate) fn html(path: &str, bytes: &[u8], raw_href: &str) -> Option<String> {
    if !is_font(path) {
        return None;
    }
    let info = std::panic::catch_unwind(|| read(bytes)).ok().flatten()?;
    FontMarkup {
        glyphs: crate::fmt::count(info.glyphs, "glyph", "glyphs"),
        f: &info,
        src: Some(raw_href).filter(|h| !h.is_empty()),
    }
    .render()
    .ok()
}

fn read(bytes: &[u8]) -> Option<Info> {
    if bytes.is_empty() || bytes.len() > BYTES_CAP {
        return None;
    }
    let sfnt = unpack(bytes)?;
    let font = FontRef::new(&sfnt).ok()?;
    let glyphs = u32::from(font.maxp().ok()?.num_glyphs());
    let family = name(
        &font,
        &[StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME],
    )
    .unwrap_or_else(|| "unnamed".to_owned());
    let mut styles: Vec<String> = Vec::new();
    for i in font.named_instances().iter().take(NAMES_CAP) {
        if let Some(n) = name(&font, &[i.subfamily_name_id()])
            && !styles.contains(&n)
        {
            styles.push(n);
        }
    }
    if styles.is_empty() {
        let one = name(
            &font,
            &[
                StringId::TYPOGRAPHIC_SUBFAMILY_NAME,
                StringId::SUBFAMILY_NAME,
            ],
        );
        styles.push(one.unwrap_or_else(|| format!("weight {}", font.attributes().weight.value())));
    }
    let axes: Vec<String> = font
        .axes()
        .iter()
        .take(NAMES_CAP)
        .map(|a| format!("{} {} to {}", a.tag(), a.min_value(), a.max_value()))
        .collect();
    Some(Info {
        family,
        styles: styles.join(", "),
        glyphs,
        axes: axes.join(", "),
    })
}

/// The first of `ids` the font names, English or else the first language, made safe to show.
fn name(font: &FontRef<'_>, ids: &[StringId]) -> Option<String> {
    ids.iter()
        .find_map(|id| font.localized_strings(*id).english_or_first())
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(NAME_LEN)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .filter(|s| !s.is_empty())
}

/// The sfnt inside `bytes`: a woff or woff2 unpacked under [`DECODED_CAP`], anything else as is.
fn unpack(bytes: &[u8]) -> Option<Cow<'_, [u8]>> {
    match bytes.get(..4)? {
        b"wOFF" => {
            let mut spent = 0_usize;
            let mut inflate = |data: &[u8], size: usize| -> Result<Vec<u8>, Box<dyn Error>> {
                spent = spent.saturating_add(size);
                if spent > DECODED_CAP {
                    return Err("font unpacks past the cap".into());
                }
                let mut out = Vec::with_capacity(size);
                flate2::read::ZlibDecoder::new(data)
                    .take(size as u64 + 1)
                    .read_to_end(&mut out)?;
                Ok(out)
            };
            wuff::decompress_woff1_with_custom_z(bytes, &mut inflate)
                .ok()
                .map(Cow::Owned)
        }
        b"wOF2" => wuff::decompress_woff2_with_custom_brotli(bytes, &mut unbrotli)
            .ok()
            .map(Cow::Owned),
        _ => Some(Cow::Borrowed(bytes)),
    }
}

/// One brotli block of a woff2, whose size the file declares: refused before the buffer.
fn unbrotli(data: &[u8], size: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    if size > DECODED_CAP {
        return Err("font unpacks past the cap".into());
    }
    let mut out = Vec::with_capacity(size);
    brotli_decompressor::Decompressor::new(data, 4096)
        .take(size as u64 + 1)
        .read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
#[path = "font_tests.rs"]
mod tests;
