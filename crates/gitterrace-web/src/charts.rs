//! Server-rendered SVG charts. Colors come from CSS classes mapped to the
//! tokens in `static/tokens.css`, so a theme switch needs no re-render.

use std::fmt::Write as _;

use crate::fmt::{self, esc};

const W: f64 = 960.0;
const H: f64 = 220.0;
const LEFT: f64 = 58.0;
const TOP_PAD: f64 = 10.0;
const BOTTOM: f64 = 26.0;
const LABEL_PX: f64 = 84.0;
/// Named series before folding into "other"; matches `--cat-1..7`.
const SERIES: usize = 7;
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Horizontal geometry shared by both bar charts: slot width for `n` columns.
fn slot(n: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let n = n.max(1) as f64;
    (W - LEFT) / n
}

/// Gridlines with thousands-separated tick labels; returns the SVG and the scale ceiling.
fn axis(max: u64) -> (String, f64) {
    let mut svg = String::new();
    let plot = H - TOP_PAD - BOTTOM;
    let step = nice_step(max);
    #[allow(clippy::cast_precision_loss)]
    let top = (step * max.div_ceil(step).max(1)) as f64;
    let mut v = 0;
    while (v as f64) <= top {
        #[allow(clippy::cast_precision_loss)]
        let y = TOP_PAD + plot - v as f64 / top * plot;
        let _ = write!(
            svg,
            r#"<line class="grid" x1="{LEFT}" x2="{W}" y1="{y:.1}" y2="{y:.1}"/><text class="ax" x="{:.1}" y="{:.1}" text-anchor="end">{}</text>"#,
            LEFT - 8.0,
            y + 3.5,
            fmt::thousands(v)
        );
        v += step;
    }
    (svg, top)
}

/// A 1/2/5 step giving about four gridlines.
pub(crate) fn nice_step(max: u64) -> u64 {
    let raw = (max.max(1)).div_ceil(4);
    let mut mag = 1;
    while mag * 10 <= raw {
        mag *= 10;
    }
    [1, 2, 5, 10]
        .iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(mag * 10)
}

/// Month labels thinned so neighbours never collide, centred under their column. The plot
/// keeps `LEFT` clear on the left so the first label is never clipped.
fn x_labels(keys: &[&str]) -> String {
    let sl = slot(keys.len());
    let every = (LABEL_PX / sl).ceil().max(1.0) as usize;
    let mut svg = String::new();
    for (i, k) in keys.iter().enumerate().filter(|(i, _)| i % every == 0) {
        #[allow(clippy::cast_precision_loss)]
        let cx = LEFT + i as f64 * sl + sl / 2.0;
        let (x, anchor) = if cx > W - LABEL_PX / 2.0 {
            (W, "end")
        } else {
            (cx, "middle")
        };
        let _ = write!(
            svg,
            r#"<text class="ax" x="{x:.1}" y="{:.1}" text-anchor="{anchor}">{}</text>"#,
            H - 8.0,
            esc(k)
        );
    }
    svg
}

/// Categorical class for series `k` of `named`; anything past the named ones is "other".
pub(crate) fn cat(k: usize, named: usize) -> String {
    if k >= named {
        "c8".to_owned()
    } else {
        format!("c{}", k + 1)
    }
}

fn open_svg(label: &str) -> String {
    format!(
        r#"<svg class="chart" viewBox="0 0 {W:.0} {H:.0}" role="img" aria-label="{label}" preserveAspectRatio="xMidYMid meet">"#
    )
}

/// Vertical bars, one per `(label, value)`, scaled to the full container width.
#[allow(clippy::many_single_char_names)]
pub(crate) fn bars(points: &[(String, u64)]) -> String {
    let max = points.iter().map(|p| p.1).max().unwrap_or(0);
    let (grid, top) = axis(max);
    let sl = slot(points.len());
    let bw = (sl * 0.62).min(48.0);
    let plot = H - TOP_PAD - BOTTOM;
    let mut svg = open_svg("Activity");
    svg.push_str(&grid);
    for (i, (label, v)) in points.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let h = (*v as f64 / top * plot).max(if *v > 0 { 1.0 } else { 0.0 });
        #[allow(clippy::cast_precision_loss)]
        let x = LEFT + i as f64 * sl + (sl - bw) / 2.0;
        let y = TOP_PAD + plot - h;
        let _ = write!(
            svg,
            r#"<g class="col" data-tip="{}: {}"><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/><rect class="bar" x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{h:.1}" rx="2"/></g>"#,
            esc(label),
            fmt::thousands(v),
            x - (sl - bw) / 2.0
        );
    }
    let keys: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
    svg.push_str(&x_labels(&keys));
    svg.push_str("</svg>");
    svg
}

/// Stacked columns per bucket by the `SERIES` biggest labels (rest folded into `other`),
/// with a legend. Series order is overall frequency, biggest at the bottom.
#[allow(clippy::many_single_char_names)]
pub(crate) fn stacked(buckets: &[gitterrace_index::LabelBucket]) -> String {
    let mut totals: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for b in buckets {
        for c in &b.counts {
            *totals.entry(c.label.as_str()).or_default() += c.commits;
        }
    }
    let mut order: Vec<(&str, usize)> = totals.into_iter().collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let names: Vec<&str> = order.iter().take(SERIES).map(|o| o.0).collect();
    let has_other = order.len() > SERIES;
    let cols: Vec<Vec<u64>> = buckets
        .iter()
        .map(|b| {
            let mut col = vec![0u64; names.len() + 1];
            for c in &b.counts {
                let i = names
                    .iter()
                    .position(|n| *n == c.label)
                    .unwrap_or(names.len());
                col[i] += c.commits as u64;
            }
            col
        })
        .collect();
    let max = cols
        .iter()
        .map(|c| c.iter().sum::<u64>())
        .max()
        .unwrap_or(0);
    let (grid, top) = axis(max);
    let sl = slot(cols.len());
    let bw = (sl * 0.62).min(48.0);
    let plot = H - TOP_PAD - BOTTOM;
    let mut svg = open_svg("Commit types over time");
    svg.push_str(&grid);
    for (i, col) in cols.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x = LEFT + i as f64 * sl + (sl - bw) / 2.0;
        let mut y = TOP_PAD + plot;
        let key = esc(&buckets[i].label);
        let total: u64 = col.iter().sum();
        let _ = write!(
            svg,
            r#"<g class="col" data-tip="{key}: {}"><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/>"#,
            fmt::thousands(total),
            x - (sl - bw) / 2.0
        );
        for (k, v) in col.iter().enumerate() {
            if *v == 0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let h = *v as f64 / top * plot;
            y -= h;
            let _ = write!(
                svg,
                r#"<rect class="{}" x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{h:.1}"/>"#,
                cat(k, names.len())
            );
        }
        svg.push_str("</g>");
    }
    let keys: Vec<&str> = buckets.iter().map(|b| b.label.as_str()).collect();
    svg.push_str(&x_labels(&keys));
    svg.push_str("</svg><div class=\"legend\">");
    for (k, n) in names.iter().enumerate() {
        let _ = write!(
            svg,
            r#"<span><i class="{}"></i>{}</span>"#,
            cat(k, names.len()),
            esc(n)
        );
    }
    if has_other {
        svg.push_str(r#"<span><i class="c8"></i>other</span>"#);
    }
    svg.push_str("</div>");
    svg
}

/// Additions above the axis and deletions below it, one column per bucket.
#[allow(clippy::many_single_char_names)]
pub(crate) fn diverging(points: &[(String, u64, u64)], title: &str) -> String {
    let max = points.iter().map(|p| p.1.max(p.2)).max().unwrap_or(0);
    let step = nice_step(max);
    #[allow(clippy::cast_precision_loss)]
    let top = (step * max.div_ceil(step).max(1)) as f64;
    let half = (H - TOP_PAD - BOTTOM) / 2.0;
    let mid = TOP_PAD + half;
    let mut svg = open_svg(title);
    let mut v = 0;
    while (v as f64) <= top {
        #[allow(clippy::cast_precision_loss)]
        let dy = v as f64 / top * half;
        for (sign, y) in [("", mid - dy), ("-", mid + dy)] {
            if v == 0 && sign == "-" {
                continue;
            }
            let _ = write!(
                svg,
                r#"<line class="grid" x1="{LEFT}" x2="{W}" y1="{y:.1}" y2="{y:.1}"/><text class="ax" x="{:.1}" y="{:.1}" text-anchor="end">{sign}{}</text>"#,
                LEFT - 8.0,
                y + 3.5,
                fmt::thousands(v)
            );
        }
        v += step;
    }
    let sl = slot(points.len());
    let bw = (sl * 0.8).clamp(1.0, 48.0);
    for (i, (label, a, d)) in points.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x = LEFT + i as f64 * sl + (sl - bw) / 2.0;
        #[allow(clippy::cast_precision_loss)]
        let (ha, hd) = (*a as f64 / top * half, *d as f64 / top * half);
        let _ = write!(
            svg,
            r#"<g class="col" data-tip="{}: +{} / -{}"><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{:.1}"/><rect class="up" x="{x:.1}" y="{:.1}" width="{bw:.1}" height="{ha:.1}"/><rect class="dn" x="{x:.1}" y="{mid:.1}" width="{bw:.1}" height="{hd:.1}"/></g>"#,
            esc(label),
            fmt::thousands(a),
            fmt::thousands(d),
            x - (sl - bw) / 2.0,
            half * 2.0,
            mid - ha
        );
    }
    let keys: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
    svg.push_str(&x_labels(&keys));
    svg.push_str("</svg>");
    svg
}

/// A 0..100 percent line over buckets; `None` values break the line.
pub(crate) fn percent_line(points: &[(String, Option<f64>)]) -> String {
    let plot = H - TOP_PAD - BOTTOM;
    let mut svg = open_svg("Rework share over time");
    for p in [0, 25, 50, 75, 100] {
        let y = TOP_PAD + plot - f64::from(p) / 100.0 * plot;
        let _ = write!(
            svg,
            r#"<line class="grid" x1="{LEFT}" x2="{W}" y1="{y:.1}" y2="{y:.1}"/><text class="ax" x="{:.1}" y="{:.1}" text-anchor="end">{p}%</text>"#,
            LEFT - 8.0,
            y + 3.5
        );
    }
    let sl = slot(points.len());
    // Hover columns first, so they paint under the line. Each also carries the point
    // dot, shown on hover, so the reading is marked on the line itself.
    for (i, (label, v)) in points.iter().enumerate() {
        if let Some(v) = v {
            #[allow(clippy::cast_precision_loss)]
            let x = LEFT + i as f64 * sl;
            let y = TOP_PAD + plot - v.clamp(0.0, 1.0) * plot;
            let _ = write!(
                svg,
                r#"<g class="col" data-tip="{}: {:.0}%"><rect class="hit" x="{x:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/><circle class="dot" cx="{:.1}" cy="{y:.1}" r="3.5"/></g>"#,
                esc(label),
                v * 100.0,
                x + sl / 2.0
            );
        }
    }
    let mut d = String::new();
    let mut pen = false;
    for (i, (_, v)) in points.iter().enumerate() {
        let Some(v) = v else {
            pen = false;
            continue;
        };
        #[allow(clippy::cast_precision_loss)]
        let x = LEFT + i as f64 * sl + sl / 2.0;
        let y = TOP_PAD + plot - v.clamp(0.0, 1.0) * plot;
        let _ = write!(d, "{}{x:.1} {y:.1}", if pen { "L" } else { "M" });
        pen = true;
    }
    let _ = write!(svg, r#"<path class="line" d="{d}"/>"#);
    let keys: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
    svg.push_str(&x_labels(&keys));
    svg.push_str("</svg>");
    svg
}

/// Tiny polyline of `values`, 120x22, for table rows.
pub(crate) fn sparkline(values: &[u32], class: &str) -> String {
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let n = values.len().max(2) - 1;
    let mut pts = String::new();
    for (i, v) in values.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let x = i as f64 * 120.0 / n as f64;
        let y = 20.0 - f64::from(*v) * 18.0 / f64::from(max);
        let _ = write!(pts, "{x:.1},{y:.1} ");
    }
    format!(
        r#"<svg class="spark {class}" viewBox="0 0 120 22" width="120" height="22" aria-hidden="true"><polyline points="{}"/></svg>"#,
        pts.trim_end()
    )
}

/// One dot of [`scatter`].
pub(crate) struct Dot {
    pub(crate) x: u64,
    pub(crate) y: u64,
    /// Radius driver, for example the number of authors.
    pub(crate) size: u64,
    pub(crate) href: String,
    pub(crate) title: String,
}

/// Dots on a linear plane: x along the bottom, y up the side, each dot a link.
#[allow(clippy::many_single_char_names)]
pub(crate) fn scatter(dots: &[Dot], x_title: &str) -> String {
    let xmax = dots.iter().map(|d| d.x).max().unwrap_or(0);
    // One giant file would flatten everything else: scale to the 97th percentile and pin
    // outliers to the top edge (their tooltip still has the real size).
    let mut ys: Vec<u64> = dots.iter().map(|d| d.y).collect();
    ys.sort_unstable();
    let ymax = ys
        .get(ys.len().saturating_sub(1) * 97 / 100)
        .copied()
        .unwrap_or(0);
    let smax = dots.iter().map(|d| d.size).max().unwrap_or(1).max(1);
    let (grid, ytop) = axis(ymax);
    let xstep = nice_step(xmax);
    #[allow(clippy::cast_precision_loss)]
    let xtop = (xstep * xmax.div_ceil(xstep).max(1)) as f64;
    let plot = H - TOP_PAD - BOTTOM;
    let mut svg = open_svg("Change frequency against size");
    svg.push_str(&grid);
    let mut v = 0;
    while (v as f64) <= xtop {
        #[allow(clippy::cast_precision_loss)]
        let x = LEFT + v as f64 / xtop * (W - LEFT - 6.0);
        let _ = write!(
            svg,
            r#"<text class="ax" x="{x:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
            H - 8.0,
            fmt::thousands(v)
        );
        v += xstep;
    }
    for d in dots {
        #[allow(clippy::cast_precision_loss)]
        let (cx, cy, r) = (
            LEFT + d.x as f64 / xtop * (W - LEFT - 6.0),
            TOP_PAD + plot - (d.y as f64 / ytop).min(1.0) * plot,
            3.0 + 5.0 * (d.size as f64 / smax as f64).sqrt(),
        );
        let _ = write!(
            svg,
            r#"<a href="{}"><circle class="dot-pt" cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" data-tip="{}"/></a>"#,
            esc(&d.href),
            esc(&d.title)
        );
    }
    let _ = write!(
        svg,
        r#"<text class="ax" x="{W}" y="{:.1}" text-anchor="end">{}</text>"#,
        TOP_PAD + 2.0,
        esc(x_title)
    );
    svg.push_str("</svg>");
    svg
}

/// Weekday by hour grid. Cell intensity is bucketed into five levels.
pub(crate) fn heatmap(cells: &[[u32; 24]; 7], max: u32) -> String {
    // Sized near the bar charts' width so labels render at the same pixel size.
    const CELL: usize = 38;
    const LEFT: usize = 40;
    const TOP: usize = 20;
    let w = LEFT + 24 * CELL;
    let h = TOP + 7 * CELL;
    let mut svg = format!(
        r#"<svg class="chart heat" viewBox="0 0 {w} {h}" role="img" aria-label="Commits by weekday and hour">"#
    );
    for hour in (0..24).step_by(3) {
        let _ = write!(
            svg,
            r#"<text class="ax" x="{}" y="12">{hour:02}</text>"#,
            LEFT + hour * CELL
        );
    }
    for (d, row) in cells.iter().enumerate() {
        let y = TOP + d * CELL;
        let _ = write!(
            svg,
            r#"<text class="ax" x="0" y="{}">{}</text>"#,
            y + 22,
            DAYS[d]
        );
        for (hr, v) in row.iter().enumerate() {
            let level = if *v == 0 || max == 0 {
                0
            } else {
                (*v * 4).div_ceil(max).min(4)
            };
            let _ = write!(
                svg,
                r#"<rect class="h{level}" x="{}" y="{y}" width="{}" height="{}" rx="4" data-tip="{} {hr:02}:00, {}"/>"#,
                LEFT + hr * CELL,
                CELL - 4,
                CELL - 4,
                DAYS[d],
                crate::fmt::count(*v, "commit", "commits")
            );
        }
    }
    svg.push_str(
        r#"</svg><div class="heat-legend">Less<i></i><i></i><i></i><i></i><i></i>More</div>"#,
    );
    svg
}

/// Width percentage of `v` against `max`, for CSS bars.
pub(crate) fn pct(v: u64, max: u64) -> u32 {
    if max == 0 {
        return 0;
    }
    #[allow(clippy::cast_possible_truncation)]
    let p = (v.saturating_mul(100) / max) as u32;
    p.max(u32::from(v > 0))
}

#[cfg(test)]
mod tests {
    use crate::charts::{bars, diverging, heatmap, pct, percent_line};

    #[test]
    fn bars_escape_labels_and_scale() {
        let svg = bars(&[("<a>".to_owned(), 5), ("b".to_owned(), 0)]);
        assert!(svg.contains("&lt;a&gt;"));
        assert!(!svg.contains("<a>"));
        assert_eq!(svg.matches(r#"class="bar""#).count(), 2);
    }

    #[test]
    fn heatmap_has_all_cells() {
        let mut cells = [[0; 24]; 7];
        cells[0][0] = 3;
        let svg = heatmap(&cells, 3);
        assert_eq!(svg.matches("<rect").count(), 168);
        assert!(svg.contains(r#"class="h4""#));
    }

    #[test]
    fn pct_keeps_tiny_values_visible() {
        assert_eq!(pct(1, 1000), 1);
        assert_eq!(pct(0, 10), 0);
        assert_eq!(pct(5, 10), 50);
    }

    /// A hover band is opaque on hover; drawn after a mark it hides the mark. In every
    /// chart, the last hover band must come before the first data mark it could cover.
    #[test]
    fn hover_bands_paint_under_the_data() {
        let line = percent_line(&[
            ("a".to_owned(), Some(0.2)),
            ("b".to_owned(), Some(0.5)),
            ("c".to_owned(), None),
        ]);
        let last_hit = line.rfind(r#"class="hit""#).expect("hit");
        let path = line.find(r#"class="line""#).expect("line");
        assert!(last_hit < path, "hover band drawn over the line");
        for svg in [
            bars(&[("a".to_owned(), 3), ("b".to_owned(), 1)]),
            diverging(&[("a".to_owned(), 3, 1)], "t"),
        ] {
            for col in svg.split(r#"<g class="col">"#).skip(1) {
                let hit = col.find(r#"class="hit""#).expect("hit");
                let mark = col
                    .find(r#"class="bar""#)
                    .or_else(|| col.find(r#"class="up""#))
                    .expect("mark");
                assert!(hit < mark, "hover band drawn over a bar");
            }
        }
    }
}
