//! The app-icon plate: a dark rounded square behind the mark, for the places that need an
//! icon of their own (the org avatar, the touch icon, the favicon on a light tab strip).
//! The mark's paths live once, in `docs/brand/mark-dark.svg`; every plated file is built
//! from them here, so `check` fails when one drifts.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::out::Out;

/// The plate color, `rgb(33,35,38)`.
pub const PLATE_COLOR: &str = "#212326";
/// The plate corner radius in a 32 unit box (22.5%).
pub const PLATE_RADIUS: &str = "7.2";
/// The mark on the plate: 69% of the box, centered.
const MARK_PLACE: &str = "translate(5 5) scale(0.6875)";

const HEAD: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" role=\"img\" \
                    aria-label=\"gitterrace\" viewBox=\"0 0 32 32\" width=\"32\" height=\"32\">\
                    <title>gitterrace</title>";

/// The mark's `<g>` element, taken from the dark mark file.
fn mark_paths(dark_svg: &str) -> Result<&str> {
    let start = dark_svg.find("</title>").context("mark-dark.svg title")? + "</title>".len();
    let end = dark_svg.rfind("</svg>").context("mark-dark.svg end")?;
    Ok(&dark_svg[start..end])
}

fn plate_rect(color: &str, class: &str) -> String {
    format!("<rect{class} width=\"32\" height=\"32\" rx=\"{PLATE_RADIUS}\" fill=\"{color}\"/>")
}

/// `mark-plate.svg`: the plated mark.
pub fn plated(dark_svg: &str, color: &str) -> Result<String> {
    let g = mark_paths(dark_svg)?;
    Ok(format!("{HEAD}{}<g transform=\"{MARK_PLACE}\">{g}</g></svg>\n", plate_rect(color, "")))
}

/// The favicon: the plate in light mode, the bare light-on-dark mark in dark mode. The mark
/// keeps its colors in both, so the media query only drops the plate and the placement.
pub fn favicon(dark_svg: &str, color: &str) -> Result<String> {
    let g = mark_paths(dark_svg)?;
    Ok(format!(
        "{HEAD}<style>@media (prefers-color-scheme:dark){{.p{{display:none}}\
         .m{{transform:none}}}}</style>{}<g class=\"m\" transform=\"{MARK_PLACE}\">{g}</g></svg>\n",
        plate_rect(color, " class=\"p\"")
    ))
}

/// The favicon as the `<link>` the app puts in every page head.
pub fn favicon_link(svg: &str) -> String {
    let mut uri = String::new();
    for c in svg.trim_end().chars() {
        match c {
            '%' => uri.push_str("%25"),
            '"' => uri.push_str("%22"),
            '<' => uri.push_str("%3C"),
            '>' => uri.push_str("%3E"),
            '#' => uri.push_str("%23"),
            '{' => uri.push_str("%7B"),
            '}' => uri.push_str("%7D"),
            '@' => uri.push_str("%40"),
            c => uri.push(c),
        }
    }
    format!("<link rel=\"icon\" href=\"data:image/svg+xml,{uri}\">\n")
}

/// Every plate-derived file, for the plate color `color`.
pub fn outs(root: &Path, color: &str) -> Result<Vec<Out>> {
    let dark = std::fs::read_to_string(root.join("docs/brand/mark-dark.svg"))
        .context("reading docs/brand/mark-dark.svg")?;
    let fav = favicon(&dark, color)?;
    Ok(vec![
        Out::file("docs/brand/mark-plate.svg", plated(&dark, color)?),
        Out::file("crates/gitterrace-web/templates/_favicon.html", favicon_link(&fav)),
        Out::file("docs/brand/mark.svg", fav),
    ])
}

#[cfg(test)]
mod tests;
