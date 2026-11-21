//! The plain logos in `docs/brand` and the inline mark and wordmark in every page header, drawn
//! from the README headers. The header SVGs are the single source: the terrace mark, the
//! outlined "git" and "terrace", then the tagline (see `tagline`). This keeps the first three
//! and drops the tagline for `logo-{light,dark}.svg`, and writes the mark alone, square, as
//! `mark-{light,dark}.svg`. The app and the site draw the mark and the wordmark inline, colored
//! by the page's tokens.

#![expect(
    clippy::suboptimal_flops,
    reason = "the arithmetic must round exactly as the reference did, no fused multiply-add"
)]

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use regex::{Captures, Regex};

use crate::out::Out;
use crate::tagline;
use crate::theme::contrast;

const BRAND: &str = "docs/brand";
const TEMPLATES: &str = "crates/gitterrace-web/templates";
const SITE_TEMPLATES: &str = "xtask/templates";
const TOKENS: &str = "crates/gitterrace-web/static/tokens.css";
/// The header's wordmark ends near x=545 and its tagline starts below y=128; the mark's group
/// draws the 32 grid scaled by 3.2 from (40, 40), so the 28-unit stair box spans y 46 to 136,
/// the wordmark's height from the top of "t" to the tail of "g".
const LOGO_BOX: (u32, u32, u32, u32) = (24, 18, 540, 120);
const MARK_AT: &str = "transform=\"translate(40 40) scale(3.2)\"";
/// The wordmark's two halves must stay visibly apart in every theme.
const MIN_SPLIT: f64 = 3.0;
/// The step-in mark: a plus-shaped stem and arm on the 32 grid, then a tread under it, and below
/// a separate accent bar. Every arm is ARM thick, the stem's centre is at (CENTRE, CENTRE), the
/// tread ends at `TREAD_X` and its underside sits at `TREAD_Y`. The bar starts `BAR_GAP` under the
/// tread, is ARM tall and runs from the stem's left edge to x=30. Every convex corner is rounded
/// by CONVEX and every concave one by CONCAVE. The ink and the accent are two shapes with clear
/// air between them, so there is no shared edge to seam.
const ARM: f64 = 4.5;
const CENTRE: f64 = 11.0;
const TREAD_X: f64 = 24.5;
const TREAD_Y: f64 = 23.5;
const BAR_GAP: f64 = 1.5;
const EDGE_LO: f64 = 2.0;
const EDGE_HI: f64 = 20.0;
const BAR_END: f64 = 30.0;
const CONVEX: f64 = 1.25;
const CONCAVE: f64 = 0.75;
/// The tokens that color the inline wordmark, each the header's fill for its half.
const WORD_TOKENS: [&str; 2] = ["--wm-git", "--wm-terrace"];

type Point = (f64, f64);

/// The patterns, compiled once per run.
struct Re {
    mark_group: Regex,
    paths: Regex,
    app_logo: Regex,
    wordmark: Regex,
    fill: Regex,
}

impl Re {
    fn new() -> Result<Self> {
        Ok(Self {
            mark_group: Regex::new(&format!("(?s)<g {}>.*?</g>", regex::escape(MARK_AT)))?,
            paths: Regex::new(
                r##"<path fill="#[0-9a-f]{6}"(?: data-[a-z]+="[^"]*")* d="[^"]*"/>"##,
            )?,
            app_logo: Regex::new(r#"(?s)<svg class="logo" [^>]*>.*?</svg>"#)?,
            wordmark: Regex::new(r#"(?s)<svg class="wordmark" [^>]*>.*?</svg>"#)?,
            fill: Regex::new(r#"fill="(#[0-9a-f]{6})""#)?,
        })
    }

    /// The text paths of a header, outside the mark's group (which draws a path of its own).
    fn text_paths(&self, header: &str) -> Vec<String> {
        let bare = self.mark_group.replace_all(header, "");
        self.paths.find_iter(&bare).map(|m| m.as_str().to_owned()).collect()
    }
}

/// A coordinate with at most three decimals and no trailing zeros, so output is stable.
fn num(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.').replace("-0", "0");
    if s.is_empty() { "0".to_owned() } else { s }
}

/// A clockwise rectilinear polygon as one path, corners rounded unless `round` is off.
fn outline(pts: &[Point], scale: f64, round: bool) -> String {
    let n = pts.len();
    let mut segs: Vec<String> = Vec::new();
    for i in 0..n {
        let (p0, p1, p2) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        let (d1, d2) = ((p1.0 - p0.0, p1.1 - p0.1), (p2.0 - p1.0, p2.1 - p1.1));
        let (l1, l2) = (d1.0.hypot(d1.1), d2.0.hypot(d2.1));
        let (u1, u2) = ((d1.0 / l1, d1.1 / l1), (d2.0 / l2, d2.1 / l2));
        let convex = u1.0 * u2.1 - u1.1 * u2.0 > 0.0;
        let corner = if !round {
            0.0
        } else if convex {
            CONVEX
        } else {
            CONCAVE
        };
        let r = corner.min(l1 / 2.0).min(l2 / 2.0);
        let a = (p1.0 - u1.0 * r, p1.1 - u1.1 * r);
        let b = (p1.0 + u2.0 * r, p1.1 + u2.1 * r);
        let cmd = if i == 0 { 'M' } else { 'L' };
        segs.push(format!("{cmd}{} {}", num(a.0 * scale), num(a.1 * scale)));
        if r != 0.0 {
            segs.push(format!(
                "A{} {} 0 0 {} {} {}",
                num(r * scale),
                num(r * scale),
                u8::from(convex),
                num(b.0 * scale),
                num(b.1 * scale)
            ));
        }
    }
    segs.concat() + "Z"
}

/// The ink and accent outlines.
fn halves() -> (String, String) {
    let h = ARM / 2.0;
    let c = CENTRE;
    let (s0, s1, c0, c1) = (c - h, c + h, c - h, c + h);
    let tread_top = TREAD_Y - ARM;
    let ink = [
        (s0, TREAD_Y),
        (s0, c1),
        (EDGE_LO, c1),
        (EDGE_LO, c0),
        (s0, c0),
        (s0, EDGE_LO),
        (s1, EDGE_LO),
        (s1, c0),
        (EDGE_HI, c0),
        (EDGE_HI, c1),
        (s1, c1),
        (s1, tread_top),
        (TREAD_X, tread_top),
        (TREAD_X, TREAD_Y),
    ];
    let top = TREAD_Y + BAR_GAP;
    let bar = [(s0, top), (BAR_END, top), (BAR_END, top + ARM), (s0, top + ARM)];
    (outline(&ink, 1.0, true), outline(&bar, 1.0, true))
}

fn ink(theme: &str) -> (&'static str, &'static str) {
    if theme == "light" { ("#21272e", "#5b5e8d") } else { ("#e3ecf6", "#9ca1d4") }
}

/// The mark as the README header draws it, in `theme`'s colors.
fn mark_group(theme: &str) -> String {
    let ((ink, acc), (top, bottom)) = (ink(theme), halves());
    format!(
        "<g {MARK_AT}><path fill=\"{ink}\" d=\"{top}\"/><path fill=\"{acc}\" d=\"{bottom}\"/></g>"
    )
}

/// The mark in the app's page header, colored by the page's tokens through `app.css`.
fn app_logo() -> String {
    let (top, bottom) = halves();
    format!(
        "<svg class=\"logo\" viewBox=\"2 2 28 28\" aria-hidden=\"true\">\
         <path class=\"i\" d=\"{top}\"/><path class=\"a\" d=\"{bottom}\"/></svg>"
    )
}

fn numbers(args: &str) -> Result<Vec<f64>> {
    let n = Regex::new(r"-?\d*\.?\d+(?:e-?\d+)?")?;
    n.find_iter(args).map(|m| m.as_str().parse::<f64>().map_err(Into::into)).collect()
}

/// The box of every point a path of absolute M, L, H, V, Q and Z commands names.
fn bbox(d: &str) -> Result<(f64, f64, f64, f64)> {
    let (mut xs, mut ys): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    let cmds = Regex::new(r"([MLHVQZ])([^MLHVQZ]*)")?;
    for c in cmds.captures_iter(d) {
        let v = numbers(&c[2])?;
        match &c[1] {
            "H" => xs.extend(v),
            "V" => ys.extend(v),
            _ => {
                xs.extend(v.iter().step_by(2));
                ys.extend(v.iter().skip(1).step_by(2));
            }
        }
    }
    if xs.is_empty() || ys.is_empty() || d.replace("e-", "").chars().any(|c| c.is_ascii_lowercase())
    {
        bail!("header layout changed: the wordmark is not absolute M L H V Q Z");
    }
    let min = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Ok((min(&xs), min(&ys), max(&xs), max(&ys)))
}

/// The path data of one outlined path element.
fn data(path: &str) -> Result<&str> {
    let start = path.find(" d=\"").context("header layout changed: a path without data")? + 4;
    let end = path.rfind('"').context("path end")?;
    Ok(&path[start..end])
}

/// A coordinate in hundredths as the shortest SVG number: no leading zero, no trailing zeros.
fn hundredths(v: i64) -> String {
    let (sign, a) = (if v < 0 { "-" } else { "" }, v.abs());
    let (whole, frac) = (a / 100, a % 100);
    if frac == 0 {
        return format!("{sign}{whole}");
    }
    let frac = format!("{frac:02}");
    let whole = if whole == 0 { String::new() } else { whole.to_string() };
    format!("{sign}{whole}.{}", frac.trim_end_matches('0'))
}

/// Absolute M, L, H, V, Q and Z data of at most two decimals as relative commands with the
/// separators SVG lets go. The arithmetic is in whole hundredths, so the shapes do not move.
fn relative(d: &str) -> Result<String> {
    let cmds = Regex::new(r"([MLHVQZ])([^MLHVQZ]*)")?;
    let (mut cur, mut start) = ((0_i64, 0_i64), (0_i64, 0_i64));
    let (mut out, mut last) = (String::new(), ' ');
    let push = |out: &mut String, n: i64| {
        let t = hundredths(n);
        let tail = out.rsplit(|c: char| c.is_ascii_alphabetic() || c == ' ' || c == '-').next();
        let tight = out.ends_with(|c: char| c.is_ascii_alphabetic())
            || t.starts_with('-')
            || (t.starts_with('.') && tail.is_some_and(|x| x.contains('.')));
        if !out.is_empty() && !tight {
            out.push(' ');
        }
        out.push_str(&t);
    };
    for c in cmds.captures_iter(d) {
        let nums = numbers(&c[2])?
            .into_iter()
            .map(|v| format!("{v:.2}").replace('.', "").parse::<i64>().map_err(Into::into))
            .collect::<Result<Vec<i64>>>()?;
        let (cmd, width) = match &c[1] {
            "M" | "L" => (c[1].to_lowercase(), 2),
            "H" | "V" => (c[1].to_lowercase(), 1),
            "Q" => ("q".to_owned(), 4),
            _ => {
                out.push('z');
                (cur, last) = (start, 'z');
                continue;
            }
        };
        for (i, ch) in nums.chunks(width).enumerate() {
            if ch.len() != width {
                bail!("header layout changed: a wordmark command with missing numbers");
            }
            let letter =
                if &c[1] == "M" && i > 0 { 'l' } else { cmd.chars().next().unwrap_or('l') };
            if letter != last || letter == 'm' {
                out.push(letter);
            }
            last = letter;
            match &c[1] {
                "H" => {
                    push(&mut out, ch[0] - cur.0);
                    cur.0 = ch[0];
                }
                "V" => {
                    push(&mut out, ch[0] - cur.1);
                    cur.1 = ch[0];
                }
                _ => {
                    let mut at = cur;
                    for pair in ch.chunks(2) {
                        push(&mut out, pair[0] - cur.0);
                        push(&mut out, pair[1] - cur.1);
                        at = (pair[0], pair[1]);
                    }
                    cur = at;
                    if &c[1] == "M" && i == 0 {
                        start = cur;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The outlined "git" and "terrace" as one inline SVG cropped to their ink, each half colored
/// by a class that `app.css` and `site.css` fill from the tokens.
fn wordmark(re: &Re, header: &str) -> Result<String> {
    let paths = re.text_paths(header);
    // Two decimals are far under a pixel at any header size and halve the inline bytes.
    let long = Regex::new(r"\d+\.\d{3,}")?;
    let mut ds = Vec::new();
    for p in paths.iter().take(2) {
        let d = long.replace_all(data(p)?, |m: &Captures| {
            let v: f64 =
                format!("{:.2}", m[0].parse::<f64>().unwrap_or(0.0)).parse().unwrap_or(0.0);
            num(v)
        });
        ds.push(d.into_owned());
    }
    let boxes = ds.iter().map(|d| bbox(d)).collect::<Result<Vec<_>>>()?;
    let lo = |f: fn(&(f64, f64, f64, f64)) -> f64| {
        boxes.iter().map(f).fold(f64::INFINITY, f64::min).floor()
    };
    let hi = |f: fn(&(f64, f64, f64, f64)) -> f64| {
        boxes.iter().map(f).fold(f64::NEG_INFINITY, f64::max).ceil()
    };
    let (x0, y0, x1, y1) = (lo(|b| b.0), lo(|b| b.1), hi(|b| b.2), hi(|b| b.3));
    let [git, terrace] = ds.as_slice() else { bail!("header layout changed: no wordmark paths") };
    let (git, terrace) = (relative(git)?, relative(terrace)?);
    Ok(format!(
        "<svg class=\"wordmark\" viewBox=\"{x0} {y0} {} {}\" aria-hidden=\"true\">\
         <path class=\"w-git\" d=\"{git}\"/><path class=\"w-terrace\" d=\"{terrace}\"/></svg>",
        x1 - x0,
        y1 - y0
    ))
}

/// Each wordmark token against the fill the header gives its half in `theme`. A token is a
/// plain color, a light-dark pair or a `var()` of another role, resolved here.
fn token_drift(re: &Re, header: &str, theme: &str, tokens: &str) -> Result<Vec<String>> {
    fn value(tokens: &str, theme: &str, name: &str) -> Result<String> {
        let find = Regex::new(&format!(r"{}:\s*([^;]+);", regex::escape(name)))?;
        let Some(m) = find.captures(tokens) else {
            return Ok(String::new());
        };
        let v = m[1].trim().to_owned();
        let var = Regex::new(r"^var\((--[\w-]+)\)$")?;
        if let Some(r) = var.captures(&v) {
            return value(tokens, theme, &r[1]);
        }
        let pair = Regex::new(r"^light-dark\((#[0-9a-f]{6}),\s*(#[0-9a-f]{6})\)$")?;
        if let Some(p) = pair.captures(&v) {
            return Ok(p[if theme == "light" { 1 } else { 2 }].to_owned());
        }
        Ok(v)
    }
    let mut bad = Vec::new();
    for (name, p) in WORD_TOKENS.iter().zip(re.text_paths(header)) {
        let fill = re.fill.captures(&p).context("path fill")?[1].to_owned();
        let got = value(tokens, theme, name)?;
        if got != fill {
            let shown = if got.is_empty() { "missing" } else { &got };
            bad.push(format!(
                "{TOKENS}: {name} is {shown} in {theme}, the {theme} README header fills it {fill}"
            ));
        }
    }
    Ok(bad)
}

/// A word wider than `width`, cut before command letters and minus signs, where SVG data reads
/// the same with a space. Other words come back whole.
fn split_long(word: &str, width: usize) -> Vec<&str> {
    if word.len() <= width {
        return vec![word];
    }
    let mut parts = Vec::new();
    let (mut from, mut last) = (0, 0);
    let ends = word.char_indices().skip(1).filter(|(_, c)| c.is_ascii_alphabetic() || *c == '-');
    for at in ends.map(|(i, _)| i).chain([word.len()]) {
        if at - from > width && last > from {
            parts.push(&word[from..last]);
            from = last;
        }
        last = at;
    }
    parts.push(&word[from..]);
    parts
}

/// `svg` broken at spaces into lines at most `width` wide, for the site's templates, whose
/// built pages are held to that width. A newline in a path's data reads as a space.
fn wrap(svg: &str, width: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in svg.split(' ').flat_map(|w| split_long(w, width)) {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
            word.clone_into(&mut line);
        } else if line.is_empty() {
            word.clone_into(&mut line);
        } else {
            line = format!("{line} {word}");
        }
    }
    lines.push(line);
    lines.join("\n")
}

/// The plain logo and the square mark, from a header already in `theme`'s colors.
fn build(re: &Re, header: &str) -> Result<(String, String)> {
    let mark = re.mark_group.find(header).map(|m| m.as_str());
    let paths = re.text_paths(header);
    let (Some(mark), 3) = (mark, paths.len()) else {
        bail!("header layout changed: expected the mark group and 3 text paths");
    };
    let fills = paths
        .iter()
        .take(2)
        .map(|p| re.fill.captures(p).map(|c| c[1].to_owned()).context("path fill"))
        .collect::<Result<Vec<_>>>()?;
    let split = contrast(&fills[0], &fills[1])?;
    if split < MIN_SPLIT {
        bail!(
            "'git' {} and 'terrace' {} differ by {split:.2}:1, under {MIN_SPLIT}:1, so the two \
             halves of the wordmark read as one color",
            fills[0],
            fills[1]
        );
    }
    let (x, y, w, h) = LOGO_BOX;
    let head = "<svg xmlns=\"http://www.w3.org/2000/svg\" role=\"img\" aria-label=\"gitterrace\"";
    let logo = format!(
        "{head} viewBox=\"{x} {y} {w} {h}\" width=\"{w}\" height=\"{h}\">\
         <title>gitterrace</title>{mark}{}{}</svg>\n",
        paths[0], paths[1]
    );
    let inner = mark.replace(&format!(" {MARK_AT}"), "");
    let square = format!(
        "{head} viewBox=\"0 0 32 32\" width=\"32\" height=\"32\">\
         <title>gitterrace</title>{inner}</svg>\n"
    );
    Ok((logo, square))
}

fn pages(root: &Path) -> Result<Vec<String>> {
    let mut all = Vec::new();
    for dir in [TEMPLATES, SITE_TEMPLATES] {
        for name in crate::out::files_in(&root.join(dir), "html")? {
            all.push(format!("{dir}/{name}"));
        }
    }
    all.sort();
    Ok(all)
}

/// Every file derived from the README headers, and a failure for each layout rule broken.
pub fn outs(root: &Path) -> Result<Vec<Out>> {
    let re = Re::new()?;
    let read =
        |p: &str| std::fs::read_to_string(root.join(p)).with_context(|| format!("reading {p}"));
    let tokens = read(TOKENS)?;
    let mut outs = Vec::new();
    let mut problems = Vec::new();
    let mut word = String::new();
    for theme in ["light", "dark"] {
        let path = format!("{BRAND}/readme-header-{theme}.svg");
        let header = read(&path)?;
        if !re.mark_group.is_match(&header) {
            bail!("{path}: no mark group {MARK_AT}");
        }
        let marked = re.mark_group.replace(&header, |_: &Captures| mark_group(theme)).into_owned();
        tagline::verify(&marked).with_context(|| path.clone())?;
        let header = marked;
        let (logo, square) = build(&re, &header)?;
        problems.extend(token_drift(&re, &header, theme, &tokens)?);
        if theme == "light" {
            // The halves have the same shapes in both themes, only the color changes.
            word = wordmark(&re, &header)?;
        }
        outs.push(Out::file(path, header));
        outs.push(Out::file(format!("{BRAND}/logo-{theme}.svg"), logo));
        outs.push(Out::file(format!("{BRAND}/mark-{theme}.svg"), square));
    }
    // Every page header, the app's and the site's, draws the same mark and wordmark inline.
    for page in pages(root)? {
        let text = read(&page)?;
        let has = re.app_logo.is_match(&text) || re.wordmark.is_match(&text);
        if text.contains("class=\"brand\"") && !re.wordmark.is_match(&text) {
            problems.push(format!(
                "{page}: its header brand has no <svg class=\"wordmark\">, so it shows a text name"
            ));
        }
        // Every template that writes its own <head> takes the icon from the include, so a new
        // page shell cannot ship without a favicon.
        if page.starts_with(TEMPLATES)
            && text.contains("<head>")
            && !text.contains("{% include \"_favicon.html\" %}")
        {
            problems
                .push(format!("{page}: has a <head> without {{% include \"_favicon.html\" %}}"));
        }
        if has {
            let fit = |s: String| {
                if page.starts_with(SITE_TEMPLATES) { wrap(&s, 100) } else { s }
            };
            let text = re.app_logo.replace_all(&text, |_: &Captures| fit(app_logo())).into_owned();
            let text =
                re.wordmark.replace_all(&text, |_: &Captures| fit(word.clone())).into_owned();
            outs.push(Out::file(page, text));
        }
    }
    if !problems.is_empty() {
        bail!("the logo layout rules fail:\n  {}", problems.join("\n  "));
    }
    Ok(outs)
}

#[cfg(test)]
mod tests;
