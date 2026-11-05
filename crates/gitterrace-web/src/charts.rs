//! Server-rendered SVG charts. Colors come from CSS classes mapped to the
//! tokens in `static/tokens.css`, so a theme switch needs no re-render.

use std::fmt::Write as _;

use crate::charttip::{self, COMMITS, Unit};
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
    let n = n.max(1) as f64;
    (W - LEFT) / n
}

/// Gridlines with thousands-separated tick labels; returns the SVG and the scale ceiling.
fn axis(max: u64) -> (String, f64) {
    let mut svg = String::new();
    let plot = H - TOP_PAD - BOTTOM;
    let (step, top_n) = ticks(max, MAX_TICKS);
    let top = top_n as f64;
    let mut v = 0;
    while v <= top_n {
        let y = (v as f64 / top).mul_add(-plot, TOP_PAD + plot);
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

/// Most value ticks a full-height axis shows, zero included. Not configurable: it follows the
/// chart's fixed height, where more labels would collide.
const MAX_TICKS: u64 = 5;

/// Ticks for values `0..=max`: a step of 1, 2 or 5 times a power of ten, and the top tick, the
/// first multiple of the step at or above `max`. At most `n` ticks, zero included, so the axis
/// stays readable; never fewer than two, so a range always shows its scale.
pub(crate) fn ticks(max: u64, n: u64) -> (u64, u64) {
    let n = n.max(2);
    let max = max.max(1);
    let mut mag = 1u64;
    loop {
        for m in [1, 2, 5] {
            let step = m * mag;
            let top = step * max.div_ceil(step);
            if top / step < n {
                return (step, top);
            }
        }
        mag = mag.saturating_mul(10);
    }
}

/// Bucket labels thinned so neighbours never collide, centred under their column. The plot
/// keeps `LEFT` clear on the left so the first label is never clipped. The spacing is a calendar
/// step: quarters every 1, 2 or 4 (a year, on Q1), other buckets every 1, 2 or 5 times a power of
/// ten.
fn x_labels(keys: &[&str]) -> String {
    let sl = slot(keys.len());
    let need = (LABEL_PX / sl).ceil().max(1.0) as usize;
    let quarters = keys.first().is_some_and(|k| k.starts_with('Q'));
    let every = label_every(need, quarters);
    // Quarter labels at a year step sit on Q1, so each one names a year.
    let start = if quarters && every >= 4 {
        keys.iter().position(|k| k.starts_with("Q1")).unwrap_or(0) % every
    } else {
        0
    };
    let mut svg = String::new();
    for (i, k) in keys
        .iter()
        .enumerate()
        .filter(|(i, _)| *i >= start && (i - start) % every == 0)
    {
        let cx = (i as f64).mul_add(sl, LEFT) + sl / 2.0;
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

/// The label spacing at or above `need` columns: a whole number of years for quarters, a nice
/// number for the rest.
pub(crate) fn label_every(need: usize, quarters: bool) -> usize {
    let need = need.max(1);
    if quarters {
        return match need {
            1 => 1,
            2 => 2,
            3 | 4 => 4,
            n => 4 * n.div_ceil(4),
        };
    }
    let mut mag = 1usize;
    loop {
        for m in [1, 2, 5] {
            if m * mag >= need {
                return m * mag;
            }
        }
        mag *= 10;
    }
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

/// [`open_svg`] for a chart whose marks are links: the chart is one tab stop and the arrow keys
/// walk its marks (app.js), so a year of weekly bars is not fifty stops on the way down the page.
fn open_linked_svg(label: &str) -> String {
    format!(
        concat!(
            r#"<svg class="chart" viewBox="0 0 {W:.0} {H:.0}" role="group" tabindex="0" "#,
            r#"data-marks aria-label="{label}, arrow keys move between marks" "#,
            r#"preserveAspectRatio="xMidYMid meet">"#
        ),
        W = W,
        H = H,
        label = label
    )
}

/// The opening tag of one linked mark: out of the tab order, reached with the arrow keys.
const MARK_LINK: &str = r#"<a tabindex="-1" href=""#;

/// Vertical bars, one per `(label, value)` counted in `unit`, scaled to the full container
/// width. A bar's tip gives its share of all bars.
pub(crate) fn bars(points: &[(String, u64)], unit: Unit) -> String {
    bars_with(points, None, unit)
}

/// [`bars`] with every bar a link to `links[i]`, for keyboard and pointer alike.
pub(crate) fn linked_bars(points: &[(String, u64)], links: &[String]) -> String {
    bars_with(points, Some(links), COMMITS)
}

fn bars_with(points: &[(String, u64)], links: Option<&[String]>, unit: Unit) -> String {
    let max = points.iter().map(|p| p.1).max().unwrap_or(0);
    let all: u64 = points.iter().map(|p| p.1).sum();
    let (grid, top) = axis(max);
    let sl = slot(points.len());
    let bw = (sl * 0.62).min(48.0);
    let plot = H - TOP_PAD - BOTTOM;
    let mut svg = if links.is_some() {
        open_linked_svg("Activity")
    } else {
        open_svg("Activity")
    };
    svg.push_str(&grid);
    for (i, (label, v)) in points.iter().enumerate() {
        let h = (*v as f64 / top * plot).max(if *v > 0 { 1.0 } else { 0.0 });
        let x = (i as f64).mul_add(sl, LEFT) + (sl - bw) / 2.0;
        let y = TOP_PAD + plot - h;
        let link = links.and_then(|l| l.get(i));
        if let Some(href) = link {
            let _ = write!(
                svg,
                r#"{MARK_LINK}{}" aria-label="{}: {}, narrow to this range">"#,
                esc(href),
                esc(label),
                fmt::count(*v, "commit", "commits")
            );
        }
        let _ = write!(
            svg,
            r#"<g class="col"{}><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/><rect class="bar" x="{x:.1}" y="{y:.1}" width="{bw:.1}" height="{h:.1}" rx="2"/></g>"#,
            charttip::attr(&charttip::part(label, *v, all, unit)),
            x - (sl - bw) / 2.0
        );
        if link.is_some() {
            svg.push_str("</a>");
        }
    }
    let keys: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
    svg.push_str(&x_labels(&keys));
    svg.push_str("</svg>");
    svg
}

/// Stacked columns per bucket by the `SERIES` biggest labels (rest folded into `other`),
/// with a legend. Series order is overall frequency, biggest at the bottom.
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
        let x = (i as f64).mul_add(sl, LEFT) + (sl - bw) / 2.0;
        let mut y = TOP_PAD + plot;
        let parts: Vec<(&str, u64)> = col
            .iter()
            .enumerate()
            .map(|(k, v)| (names.get(k).copied().unwrap_or("other"), *v))
            .collect();
        let _ = write!(
            svg,
            r#"<g class="col"{}><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/>"#,
            charttip::attr(&charttip::breakdown(&buckets[i].label, &parts, COMMITS)),
            x - (sl - bw) / 2.0
        );
        for (k, v) in col.iter().enumerate() {
            if *v == 0 {
                continue;
            }
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
pub(crate) fn diverging(points: &[(String, u64, u64)], title: &str) -> String {
    let max = points.iter().map(|p| p.1.max(p.2)).max().unwrap_or(0);
    // Each half is half the height, so it gets half the ticks: zero, then one or two a side.
    let (step, top_n) = ticks(max, 3);
    let top = top_n as f64;
    let half = (H - TOP_PAD - BOTTOM) / 2.0;
    let mid = TOP_PAD + half;
    let mut svg = open_svg(title);
    let mut v = 0;
    while v <= top_n {
        let dy = v as f64 / top * half;
        for (sign, y) in [("", mid - dy), ("\u{2212}", mid + dy)] {
            if v == 0 && !sign.is_empty() {
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
        let x = (i as f64).mul_add(sl, LEFT) + (sl - bw) / 2.0;
        let (ha, hd) = (*a as f64 / top * half, *d as f64 / top * half);
        let _ = write!(
            svg,
            r#"<g class="col"{}><rect class="hit" x="{:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{:.1}"/><rect class="up" x="{x:.1}" y="{:.1}" width="{bw:.1}" height="{ha:.1}"/><rect class="dn" x="{x:.1}" y="{mid:.1}" width="{bw:.1}" height="{hd:.1}"/></g>"#,
            charttip::attr(&charttip::breakdown(
                &format!("{label}, lines changed"),
                &[("added", *a), ("removed", *d)],
                charttip::LINES
            )),
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
        let y = (f64::from(p) / 100.0).mul_add(-plot, TOP_PAD + plot);
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
            let x = (i as f64).mul_add(sl, LEFT);
            let y = v.clamp(0.0, 1.0).mul_add(-plot, TOP_PAD + plot);
            let _ = write!(
                svg,
                r#"<g class="col"{}><rect class="hit" x="{x:.1}" y="{TOP_PAD}" width="{sl:.1}" height="{plot:.1}"/><circle class="dot" cx="{:.1}" cy="{y:.1}" r="3.5"/></g>"#,
                charttip::attr(&charttip::rate(
                    label,
                    *v,
                    "of changed lines replaced existing code"
                )),
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
        let x = (i as f64).mul_add(sl, LEFT) + sl / 2.0;
        let y = v.clamp(0.0, 1.0).mul_add(-plot, TOP_PAD + plot);
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
    let (xstep, xtop_n) = ticks(xmax, 6);
    let xtop = xtop_n as f64;
    let plot = H - TOP_PAD - BOTTOM;
    let mut svg = open_linked_svg("Change frequency against size");
    svg.push_str(&grid);
    let mut v = 0;
    while v <= xtop_n {
        let x = (v as f64 / xtop).mul_add(W - LEFT - 6.0, LEFT);
        let _ = write!(
            svg,
            r#"<text class="ax" x="{x:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
            H - 8.0,
            fmt::thousands(v)
        );
        v += xstep;
    }
    for d in dots {
        let (cx, cy, r) = (
            (d.x as f64 / xtop).mul_add(W - LEFT - 6.0, LEFT),
            (d.y as f64 / ytop).min(1.0).mul_add(-plot, TOP_PAD + plot),
            5.0f64.mul_add((d.size as f64 / smax as f64).sqrt(), 3.0),
        );
        let _ = write!(
            svg,
            r#"{MARK_LINK}{}" aria-label="{t}"><circle class="dot-pt" cx="{cx:.1}" "#,
            esc(&d.href),
            t = esc(&d.title)
        );
        let _ = write!(
            svg,
            r#"cy="{cy:.1}" r="{r:.1}"{}/></a>"#,
            charttip::attr(&d.title)
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
    let all: u64 = cells.iter().flatten().map(|v| u64::from(*v)).sum();
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
                r#"<rect class="h{level}" x="{}" y="{y}" width="{}" height="{}" rx="4"{}/>"#,
                LEFT + hr * CELL,
                CELL - 4,
                CELL - 4,
                charttip::attr(&charttip::part(
                    &format!("{} {hr:02}:00", DAYS[d]),
                    u64::from(*v),
                    all,
                    COMMITS
                ))
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
    let p = (v.saturating_mul(100) / max) as u32;
    p.max(u32::from(v > 0))
}

#[cfg(test)]
mod tests;
