//! Server-rendered SVG charts. Colors come from CSS classes so themes apply.

use crate::fmt::esc;

const WIDTH: usize = 720;
const HEIGHT: usize = 140;
const LABEL_H: usize = 18;
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Vertical bars, one per `(label, value)`. Only the first and last label are drawn.
pub(crate) fn bars(points: &[(String, u64)]) -> String {
    let max = points.iter().map(|p| p.1).max().unwrap_or(0).max(1);
    let n = points.len().max(1);
    let slot = WIDTH as f64 / n as f64;
    let plot = (HEIGHT - LABEL_H) as f64;
    let mut svg = format!(
        r#"<svg class="chart" viewBox="0 0 {WIDTH} {HEIGHT}" role="img" aria-label="Activity">"#
    );
    for (i, (label, v)) in points.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let h = (*v as f64 / max as f64 * plot).max(if *v > 0 { 1.0 } else { 0.0 });
        let x = i as f64 * slot;
        let w = (slot * 0.8).max(1.0);
        svg.push_str(&format!(
            r#"<rect x="{x:.1}" y="{:.1}" width="{w:.1}" height="{h:.1}"><title>{}: {v}</title></rect>"#,
            plot - h,
            esc(label)
        ));
    }
    if let (Some(first), Some(last)) = (points.first(), points.last()) {
        let y = HEIGHT - 4;
        svg.push_str(&format!(r#"<text x="0" y="{y}">{}</text>"#, esc(&first.0)));
        svg.push_str(&format!(
            r#"<text x="{WIDTH}" y="{y}" text-anchor="end">{}</text>"#,
            esc(&last.0)
        ));
    }
    svg.push_str("</svg>");
    svg
}

/// Weekday by hour grid. Cell intensity is bucketed into five levels.
pub(crate) fn heatmap(cells: &[[u32; 24]; 7], max: u32) -> String {
    const CELL: usize = 24;
    const LEFT: usize = 36;
    const TOP: usize = 16;
    let w = LEFT + 24 * CELL;
    let h = TOP + 7 * CELL;
    let mut svg = format!(
        r#"<svg class="chart heat" viewBox="0 0 {w} {h}" role="img" aria-label="Commits by weekday and hour">"#
    );
    for hour in (0..24).step_by(3) {
        svg.push_str(&format!(
            r#"<text x="{}" y="11">{hour:02}</text>"#,
            LEFT + hour * CELL
        ));
    }
    for (d, row) in cells.iter().enumerate() {
        let y = TOP + d * CELL;
        svg.push_str(&format!(r#"<text x="0" y="{}">{}</text>"#, y + 16, DAYS[d]));
        for (hr, v) in row.iter().enumerate() {
            let level = if *v == 0 || max == 0 {
                0
            } else {
                (*v * 4).div_ceil(max).min(4)
            };
            svg.push_str(&format!(
                r#"<rect class="h{level}" x="{}" y="{y}" width="{}" height="{}" rx="3"><title>{} {hr:02}:00, {v} commits</title></rect>"#,
                LEFT + hr * CELL,
                CELL - 2,
                CELL - 2,
                DAYS[d]
            ));
        }
    }
    svg.push_str("</svg>");
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
        assert_eq!(svg.matches("<rect").count(), 2);
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
