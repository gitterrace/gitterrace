//! SVG to PNG rasterization (feature `png`).

use crate::Error;

/// Rasterizes `svg` at its natural size. System fonts are loaded for text.
pub fn render(svg: &str) -> Result<Vec<u8>, Error> {
    render_scaled(svg, 1.0)
}

/// Like [`render`] with a pixel multiplier, e.g. 2.0 for retina badges.
pub fn render_scaled(svg: &str, scale: f32) -> Result<Vec<u8>, Error> {
    let mut opt = resvg::usvg::Options::default();
    opt.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(svg, &opt).map_err(|e| Error::Svg(e.to_string()))?;
    let size = tree
        .size()
        .to_int_size()
        .scale_by(scale)
        .ok_or_else(|| Error::Png("bad scale".to_owned()))?;
    let mut pm = resvg::tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| Error::Png("empty canvas".to_owned()))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pm.as_mut(),
    );
    pm.encode_png().map_err(|e| Error::Png(e.to_string()))
}
