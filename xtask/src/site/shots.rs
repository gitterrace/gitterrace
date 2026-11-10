//! Screenshot checks: the README's PNG files and the site's WebP files exist, are linked and are
//! the right size. Both live in `docs/screenshots/`, the one source; the site gets a copy.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::out::Out;
use crate::site::{SHOTS, at};

pub(super) const SHOT_DIR: &str = "docs/screenshots";
/// Pixel size of the lightbox WebP and of the tile thumbnail.
const FULL: (u32, u32) = (2560, 1600);
const THUMB: (u32, u32) = (1280, 800);

/// Checks the screenshots without copying them. `docs/screenshots/` holds exactly the PNG files
/// README.md links, each 2560x1600, and a full-size WebP at 2560x1600 and a 1280x800 thumbnail
/// for every entry in `SHOTS`. All of them come from
/// `make site-shots`; a missing, extra or wrongly sized file fails generation and names it.
pub(super) fn check_shots(root: &Path) -> Result<()> {
    let readme = std::fs::read_to_string(root.join("README.md")).context("reading README.md")?;
    let pngs = readme_shots(&readme);
    let all = list(root, SHOT_DIR, "make site-shots")?;
    let (webps, have): (Vec<String>, Vec<String>) = all.into_iter().partition(|f| {
        Path::new(f)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("webp"))
    });
    if let Some(f) = pngs.iter().find(|f| !have.contains(f)) {
        anyhow::bail!("{SHOT_DIR}/{f} is linked by README.md but does not exist (make site-shots)");
    }
    if let Some(f) = have.iter().find(|f| !pngs.contains(f)) {
        anyhow::bail!(
            "{SHOT_DIR}/{f} is not referenced by README.md: delete it (make site-shots keeps \
            only the README's PNG files)"
        );
    }
    for f in &pngs {
        let got = shot_size(root, f)?;
        anyhow::ensure!(
            got == (2560, 1600),
            "{SHOT_DIR}/{f} is {}x{}, the README needs 2560x1600 (make site-shots)",
            got.0,
            got.1
        );
    }
    let mut want: Vec<(String, (u32, u32))> = Vec::new();
    for (n, ..) in SHOTS {
        for c in ["light", "dark"] {
            want.push((format!("{n}-{c}.webp"), FULL));
            want.push((format!("{n}-{c}.thumb.webp"), THUMB));
        }
    }
    let have = webps;
    if let Some((f, _)) = want.iter().find(|(f, _)| !have.contains(f)) {
        anyhow::bail!("{SHOT_DIR}/{f} is missing (make site-shots)");
    }
    if let Some(f) = have.iter().find(|f| !want.iter().any(|(w, _)| w == *f)) {
        anyhow::bail!("{SHOT_DIR}/{f} is not referenced: delete it and run `make site-shots`");
    }
    for (f, size) in &want {
        let path = format!("{SHOT_DIR}/{f}");
        let got =
            webp_size(&std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?)
                .with_context(|| format!("{path} is not a WebP (make site-shots)"))?;
        anyhow::ensure!(
            got == *size,
            "{path} is {}x{}, the site needs {}x{} (make site-shots)",
            got.0,
            got.1,
            size.0,
            size.1
        );
    }
    Ok(())
}

/// The site's copy of every WebP in `SHOTS`, under `assets/screenshots/`. Run after
/// `check_shots`, so each one is there at its size.
pub(super) fn site_shots(root: &Path) -> Result<Vec<Out>> {
    let mut outs = Vec::new();
    for (n, ..) in SHOTS {
        for c in ["light", "dark"] {
            for f in [format!("{n}-{c}.webp"), format!("{n}-{c}.thumb.webp")] {
                let path = format!("{SHOT_DIR}/{f}");
                let body =
                    std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
                outs.push(Out::bytes(at(&format!("assets/screenshots/{f}")), body));
            }
        }
    }
    Ok(outs)
}

/// The `docs/screenshots/*.png` file names README.md links, sorted and deduplicated. The same
/// rule as `scripts/release/site/site-shots-run.mjs`, which copies exactly these.
pub(super) fn readme_shots(readme: &str) -> Vec<String> {
    let key = "docs/screenshots/";
    let mut v: Vec<String> = readme
        .match_indices(key)
        .filter_map(|(i, _)| {
            let rest = &readme[i + key.len()..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))?;
            let f = rest[..end].trim_end_matches('.');
            Path::new(f)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
                .then(|| f.to_string())
        })
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The file names in `dir`, sorted. A missing folder names the target that fills it.
fn list(root: &Path, dir: &str, fix: &str) -> Result<Vec<String>> {
    let mut v: Vec<String> = std::fs::read_dir(root.join(dir))
        .with_context(|| format!("reading {dir} (run `{fix}`)"))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| !f.starts_with('.'))
        .collect();
    v.sort();
    Ok(v)
}

/// Width and height in pixels of a WebP, from its first chunk: lossy (VP8), lossless (VP8L)
/// or extended (VP8X). None when the header is not one of those.
fn webp_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.len() < 30 || &b[0..4] != b"RIFF" || &b[8..12] != b"WEBP" {
        return None;
    }
    let le16 = |i: usize| u32::from(b[i]) | u32::from(b[i + 1]) << 8;
    let le24 = |i: usize| le16(i) | u32::from(b[i + 2]) << 16;
    match &b[12..16] {
        b"VP8 " if b[23..26] == [0x9d, 0x01, 0x2a] => Some((le16(26) & 0x3fff, le16(28) & 0x3fff)),
        b"VP8L" if b[20] == 0x2f => {
            let bits = u32::from_le_bytes([b[21], b[22], b[23], b[24]]);
            Some(((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1))
        }
        b"VP8X" => Some((le24(24) + 1, le24(27) + 1)),
        _ => None,
    }
}

/// Width and height in pixels of a PNG, from its header.
fn shot_size(root: &Path, file: &str) -> Result<(u32, u32)> {
    let path = format!("{SHOT_DIR}/{file}");
    let b = std::fs::read(root.join(&path)).with_context(|| format!("reading {path}"))?;
    anyhow::ensure!(b.len() > 24 && &b[1..4] == b"PNG", "{path} is not a PNG");
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    Ok((w, h))
}
