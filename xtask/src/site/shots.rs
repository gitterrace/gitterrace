//! Screenshot checks: the WebP files exist and are the right size. They live in
//! `docs/screenshots/`, the one source: the README links two of them, the site gets a copy.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::out::Out;
use crate::site::{SHOTS, at};

pub(super) const SHOT_DIR: &str = "docs/screenshots";
/// Pixel size of the lightbox WebP and of the tile thumbnail.
const FULL: (u32, u32) = (2560, 1600);
const THUMB: (u32, u32) = (1280, 800);

/// Checks the screenshots without copying them. `docs/screenshots/` holds exactly a
/// full-size WebP at 2560x1600 and a 1280x800 thumbnail for every entry in `SHOTS`, all from
/// `make site-shots`; a missing, extra or wrongly sized file fails generation and names it.
pub(super) fn check_shots(root: &Path) -> Result<()> {
    let have = list(root, SHOT_DIR, "make site-shots")?;
    let mut want: Vec<(String, (u32, u32))> = Vec::new();
    for (n, ..) in SHOTS {
        for c in ["light", "dark"] {
            want.push((format!("{n}-{c}.webp"), FULL));
            want.push((format!("{n}-{c}.thumb.webp"), THUMB));
        }
    }
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

/// The file names in `dir`, sorted. A missing folder names the target that fills it.
fn list(root: &Path, dir: &str, fix: &str) -> Result<Vec<String>> {
    let mut v: Vec<String> = std::fs::read_dir(root.join(dir))
        .with_context(|| format!("reading {dir} (run `{fix}`)"))?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|f| !f.starts_with('.') && f != "README.md")
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
