//! Server-rendered SVG charts. Colors come from CSS classes mapped to the
//! tokens in `static/tokens.css`, so a theme switch needs no re-render.

use crate::fmt::{self, esc};

const W: f64 = 960.0;
const H: f64 = 220.0;
const LEFT: f64 = 44.0;
const TOP_PAD: f64 = 10.0;
const BOTTOM: f64 = 26.0;
const LABEL_PX: f64 = 52.0;
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
        fmt::push(
            &mut svg,
            &format!(
                r#"<line class="grid" x1="{LEFT}" x2="{W}" y1="{y:.1}" y2="{y:.1}"/><text class="ax" x="{:.1}" y="{:.1}" text-anchor="end">{}</text>"#,
                LEFT - 8.0,
                y + 3.5,
                fmt::thousands(v)
            ),
        );
        v += step;
    }
    (svg, top)
}

/// A 1/2/5 step giving about four gridlines.
fn nice_step(max: u64) -> u64 {
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
        let (x, anchor) = (cx, "middle");
        fmt::push(
            &mut svg,
            &format!(
                r#"<text class="ax" x="{x:.1}" y="{:.1}" text-anchor="{anchor}">{}</text>"#,
                H - 8.0,
                esc(k)
            ),
        );
    }
    svg
}

/// Categorical class for series `k` of `named`; anything past the named ones is "other".
fn cat(k: usize, named: usize) -> String {
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
        fmt::push(
            &mut svg,
            &format!(
                r#"<g class="col"><title>{}: {}</title><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/><rect class="bar" x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{h:.1}" rx="2"/></g>"#,
                esc(label),
                fmt::thousands(v),
                x - (sl - bw) / 2.0
            ),
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
        let key = esc(&buckets[i].key);
        let total: u64 = col.iter().sum();
        fmt::push(
            &mut svg,
            &format!(
                r#"<g class="col"><title>{key}: {}</title><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/>"#,
                fmt::thousands(total),
                x - (sl - bw) / 2.0
            ),
        );
        for (k, v) in col.iter().enumerate() {
            if *v == 0 {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let h = *v as f64 / top * plot;
            y -= h;
            fmt::push(
                &mut svg,
                &format!(
                    r#"<rect class="{}" x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{h:.1}"/>"#,
                    cat(k, names.len())
                ),
            );
        }
        svg.push_str("</g>");
    }
    let keys: Vec<&str> = buckets.iter().map(|b| b.key.as_str()).collect();
    svg.push_str(&x_labels(&keys));
    svg.push_str("</svg><div class=\"legend\">");
    for (k, n) in names.iter().enumerate() {
        fmt::push(
            &mut svg,
            &format!(
                r#"<span><i class="{}"></i>{}</span>"#,
                cat(k, names.len()),
                esc(n)
            ),
        );
    }
    if has_other {
        fmt::push(&mut svg, r#"<span><i class="c8"></i>other</span>"#);
    }
    svg.push_str("</div>");
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
        fmt::push(
            &mut svg,
            &format!(
                r#"<text class="ax" x="{}" y="12">{hour:02}</text>"#,
                LEFT + hour * CELL
            ),
        );
    }
    for (d, row) in cells.iter().enumerate() {
        let y = TOP + d * CELL;
        fmt::push(
            &mut svg,
            &format!(
                r#"<text class="ax" x="0" y="{}">{}</text>"#,
                y + 22,
                DAYS[d]
            ),
        );
        for (hr, v) in row.iter().enumerate() {
            let level = if *v == 0 || max == 0 {
                0
            } else {
                (*v * 4).div_ceil(max).min(4)
            };
            fmt::push(
                &mut svg,
                &format!(
                    r#"<rect class="h{level}" x="{}" y="{y}" width="{}" height="{}" rx="4"><title>{} {hr:02}:00, {v} commits</title></rect>"#,
                    LEFT + hr * CELL,
                    CELL - 4,
                    CELL - 4,
                    DAYS[d]
                ),
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
    use crate::charts::{bars, heatmap, pct};

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
}
