//! The README header's tagline, drawn as outlined text so no font is needed to view it. The
//! words are [`TAGLINE`], set in the vendored Bricolage Grotesque at the header's size and
//! position. The path carries `data-text` (the words it spells) and `data-sha` (sha256 of its
//! path data).

#![expect(
    clippy::suboptimal_flops,
    reason = "the arithmetic must round exactly as the reference did, so no fused multiply-add"
)]

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use regex::Regex;
use sha2::{Digest as _, Sha256};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::{FontRef, MetadataProvider};

/// The one pitch sentence: the package description, the README, the landing page and the meta
/// description show it too, and generation fails when the manifest says something else.
pub const TAGLINE: &str = "Review your repo before you push, and hand findings to any tool.";

const FONT: &str =
    "vendor/schwarzwald/kit/fonts/bricolage-grotesque/bricolage-grotesque-latin-wght-normal.woff2";
const X: f64 = 174.0;
const BASELINE: f64 = 150.0;
const SIZE: f64 = 22.0;
const WEIGHT: f32 = 400.0;

/// Path data the way an SVG pen writes it: H and V for axis-aligned lines, one Q per segment,
/// every number at two decimals with no trailing zeros.
struct Pen {
    scale: f64,
    x: f64,
    cmds: Vec<String>,
    last_cmd: char,
    last: Option<(f64, f64)>,
    start: Option<(f64, f64)>,
    line_end: bool,
}

fn ntos(v: f64) -> String {
    format!("{v:.2}").trim_end_matches('0').trim_end_matches('.').to_owned()
}

impl Pen {
    fn at(&self, x: f32, y: f32) -> (f64, f64) {
        (self.scale * f64::from(x) + self.x, -self.scale * f64::from(y) + BASELINE)
    }

    fn pt((x, y): (f64, f64)) -> String {
        format!("{} {}", ntos(x), ntos(y))
    }
}

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        if self.last_cmd == 'M' {
            self.cmds.pop();
        }
        self.cmds.push(format!("M{}", Self::pt(p)));
        self.last_cmd = 'M';
        self.last = Some(p);
        self.start = Some(p);
        self.line_end = false;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        if self.last == Some(p) {
            return;
        }
        let (lx, ly) = self.last.unzip();
        let text = if Some(p.0) == lx {
            self.last_cmd = 'V';
            format!("V{}", ntos(p.1))
        } else if Some(p.1) == ly {
            self.last_cmd = 'H';
            format!("H{}", ntos(p.0))
        } else if self.last_cmd == 'M' {
            format!(" {}", Self::pt(p))
        } else {
            self.last_cmd = 'L';
            format!("L{}", Self::pt(p))
        };
        self.cmds.push(text);
        self.last = Some(p);
        self.line_end = true;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (c, p) = (self.at(x1, y1), self.at(x, y));
        self.cmds.push(format!("Q{} {}", Self::pt(c), Self::pt(p)));
        self.last_cmd = 'Q';
        self.last = Some(p);
        self.line_end = false;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x3: f32, y3: f32) {
        let (c1, c2, p) = (self.at(x1, y1), self.at(x2, y2), self.at(x3, y3));
        self.cmds.push(format!("C{} {} {}", Self::pt(c1), Self::pt(c2), Self::pt(p)));
        self.last_cmd = 'C';
        self.last = Some(p);
        self.line_end = false;
    }

    fn close(&mut self) {
        // The font's contours close implicitly, so a line back to the start is not drawn.
        if self.line_end && self.last == self.start {
            self.cmds.pop();
        }
        self.cmds.push("Z".to_owned());
        self.last_cmd = 'Z';
        self.last = None;
    }
}

/// `text` as path data, set in the vendored font at weight 400.
fn outline(root: &Path, text: &str) -> Result<String> {
    let woff = std::fs::read(root.join(FONT)).with_context(|| format!("reading {FONT}"))?;
    let ttf = wuff::decompress_woff2(&woff).context("decoding the font")?;
    let font = FontRef::new(&ttf).context("parsing the font")?;
    anyhow::ensure!(font.axes().iter().any(|a| a.tag() == "wght"), "the font has no wght axis");
    let location = font.axes().location([("wght", WEIGHT)]);
    let loc = LocationRef::from(&location);
    let scale = SIZE / f64::from(font.metrics(Size::unscaled(), loc).units_per_em);
    let charmap = font.charmap();
    let glyphs = font.outline_glyphs();
    let metrics = font.glyph_metrics(Size::unscaled(), loc);
    let mut pen = Pen {
        scale,
        x: X,
        cmds: Vec::new(),
        last_cmd: ' ',
        last: None,
        start: None,
        line_end: false,
    };
    for ch in text.chars() {
        let id = charmap.map(ch).with_context(|| format!("no glyph for {ch:?}"))?;
        glyphs
            .get(id)
            .with_context(|| format!("no outline for {ch:?}"))?
            .draw(DrawSettings::unhinted(Size::unscaled(), loc), &mut pen)
            .context("drawing a glyph")?;
        pen.x += f64::from(metrics.advance_width(id).context("advance")?) * scale;
        pen.last_cmd = ' ';
        pen.last = None;
    }
    Ok(pen.cmds.concat())
}

fn sha(d: &str) -> String {
    Sha256::digest(d.as_bytes()).iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

fn last_path(svg: &str) -> Result<regex::Captures<'_>> {
    let path = Regex::new(concat!(
        r#"<path fill="(#[0-9a-f]{6})""#,
        r#"(?: data-text="([^"]*)" data-sha="([0-9a-f]{64})")? d="([^"]*)"/>"#
    ))?;
    path.captures_iter(svg).last().context("no tagline path in the header")
}

/// Fails unless the header's last path spells [`TAGLINE`] and its digest matches its data, so
/// the outlined words cannot drift from the text. It reads the file and needs no font.
pub fn verify(svg: &str) -> Result<()> {
    let last = last_path(svg)?;
    let (text, digest, d) = (last.get(2), last.get(3), &last[4]);
    let (Some(text), Some(digest)) = (text, digest) else {
        anyhow::bail!("the tagline path has no data-text and data-sha");
    };
    ensure!(unescape(text.as_str()) == TAGLINE, "the drawn tagline is not TAGLINE");
    ensure!(digest.as_str() == sha(d), "the tagline's data-sha does not match its path");
    Ok(())
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// `svg` with its last path (the tagline, whatever it holds now) drawn again from [`TAGLINE`],
/// keeping the file's color. Run by `cargo xtask tagline` after a wording change.
pub fn redraw(root: &Path, svg: &str) -> Result<String> {
    let last = last_path(svg)?;
    let whole = last.get(0).context("match")?;
    let d = outline(root, TAGLINE)?;
    let fill = &last[1];
    Ok(format!(
        "{}<path fill=\"{fill}\" data-text=\"{}\" data-sha=\"{}\" d=\"{d}\"/>{}",
        &svg[..whole.start()],
        escape(TAGLINE),
        sha(&d),
        &svg[whole.end()..]
    ))
}

/// Draws the tagline again into both README headers. Not part of `generate`: the outline is
/// drawn in single precision, so its last digit can differ from the committed one at a tie.
pub fn rewrite(root: &Path) -> Result<Vec<String>> {
    let mut wrote = Vec::new();
    for theme in ["light", "dark"] {
        let path = format!("docs/brand/readme-header-{theme}.svg");
        let svg = std::fs::read_to_string(root.join(&path)).with_context(|| path.clone())?;
        let next = redraw(root, &svg)?;
        if next != svg {
            std::fs::write(root.join(&path), next).with_context(|| path.clone())?;
            wrote.push(path);
        }
    }
    Ok(wrote)
}

/// Fails unless `TAGLINE` is the package description, which the pages show as text.
pub fn same_as(desc: &str) -> Result<()> {
    ensure!(
        TAGLINE == desc,
        "TAGLINE in xtask/src/tagline.rs is not the package description {desc:?}: set it and \
         run `make docs-gen` to redraw the README header"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
