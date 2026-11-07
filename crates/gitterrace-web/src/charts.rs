//! Server-rendered SVG charts. Colors come from CSS classes mapped to the
//! tokens in `static/tokens.css`, so a theme switch needs no re-render.

use std::fmt::Write as _;

use crate::charttip::{self, COMMITS, Unit};
use crate::fmt;

mod text;
pub(crate) mod view;

pub(crate) use text::fig;

use view::{
    BarCol, Bars, DivCol, Diverging, Fig, Frame, HeatCell, HeatRow, Heatmap, LinePoint, P,
    PercentLine, Scatter, ScatterDot, Seg, Spark, StackCol, Stacked, Tick, XLabel,
};

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

/// Gridlines with thousands-separated tick labels; returns the ticks and the scale ceiling.
fn axis(max: u64) -> (Vec<Tick>, f64) {
    let plot = H - TOP_PAD - BOTTOM;
    let (step, top_n) = ticks(max, MAX_TICKS);
    let top = top_n as f64;
    let mut out = Vec::new();
    let mut v = 0;
    while v <= top_n {
        let y = (v as f64 / top).mul_add(-plot, TOP_PAD + plot);
        out.push(tick(y, fmt::thousands(v)));
        v += step;
    }
    (out, top)
}

fn tick(y: f64, label: String) -> Tick {
    Tick {
        y: P(y),
        ty: P(y + 3.5),
        label,
    }
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

/// Ticks for a chart with values `0..=up` above zero and `0..=down` below it: one nice step
/// shared by both sides, each side ending at its own first multiple of the step (at least one
/// step, so either side shows its scale), and at most `n` ticks in all, zero included. Returns
/// `(step, top above, top below)`.
pub(crate) fn split_ticks(up: u64, down: u64, n: u64) -> (u64, u64, u64) {
    let n = n.max(3);
    let side = |v: u64, step: u64| step * v.max(1).div_ceil(step);
    let mut mag = 1u64;
    loop {
        for m in [1, 2, 5] {
            let step = m * mag;
            let (a, d) = (side(up, step), side(down, step));
            if a / step + d / step < n {
                return (step, a, d);
            }
        }
        mag = mag.saturating_mul(10);
    }
}

/// Bucket labels thinned so neighbours never collide, centred under their column. The plot
/// keeps `LEFT` clear on the left so the first label is never clipped. The spacing is a calendar
/// step: quarters every 1, 2 or 4 (a year, on Q1), other buckets every 1, 2 or 5 times a power of
/// ten.
fn x_labels(keys: &[&str]) -> Vec<XLabel> {
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
    // Per-commit axes name a day many times over: only the first column of a run is a
    // candidate, and every k-th candidate is kept so the labels stay evenly spaced.
    let firsts = (0..keys.len()).filter(|&i| i == 0 || keys[i] != keys[i - 1]);
    let picked: Vec<usize> = if firsts.clone().count() == keys.len() {
        (start..keys.len()).step_by(every).collect()
    } else {
        let per = keys.len() as f64 / firsts.clone().count() as f64;
        let k = (need as f64 / per).ceil().max(1.0) as usize;
        firsts.step_by(k).collect()
    };
    picked
        .into_iter()
        .map(|i| (i, keys[i]))
        .map(|(i, k)| {
            let cx = (i as f64).mul_add(sl, LEFT) + sl / 2.0;
            let (x, anchor) = if cx > W - LABEL_PX / 2.0 {
                (W, "end")
            } else {
                (cx, "middle")
            };
            XLabel {
                x: P(x),
                anchor,
                text: (*k).to_owned(),
            }
        })
        .collect()
}

fn frame(fig: Fig, linked: bool, ticks: Vec<Tick>, keys: &[&str], bw: f64) -> Frame {
    Frame {
        fig,
        linked,
        sl: P(slot(keys.len())),
        bw: P(bw),
        plot: P(H - TOP_PAD - BOTTOM),
        ticks,
        labels: x_labels(keys),
    }
}

/// Vertical bars named `name`, one per `(label, value)` counted in `unit`, scaled to the full
/// container width. A bar's tip gives its share of all bars.
pub(crate) fn bars(name: &str, points: &[(String, u64)], unit: Unit) -> Bars {
    bars_with(name, points, None, unit)
}

/// [`bars`] with every bar a link to `links[i]`, for keyboard and pointer alike. A chart of
/// links is one tab stop and the arrow keys walk its marks (app.js).
pub(crate) fn linked_bars(name: &str, points: &[(String, u64)], links: &[String]) -> Bars {
    bars_with(name, points, Some(links), COMMITS)
}

fn bars_with(name: &str, points: &[(String, u64)], links: Option<&[String]>, unit: Unit) -> Bars {
    let max = points.iter().map(|p| p.1).max().unwrap_or(0);
    let all: u64 = points.iter().map(|p| p.1).sum();
    let (grid, top) = axis(max);
    let sl = slot(points.len());
    let bw = (sl * 0.62).min(48.0);
    let plot = H - TOP_PAD - BOTTOM;
    let cols = points
        .iter()
        .enumerate()
        .map(|(i, (label, v))| {
            let h = (*v as f64 / top * plot).max(if *v > 0 { 1.0 } else { 0.0 });
            let x = (i as f64).mul_add(sl, LEFT) + (sl - bw) / 2.0;
            BarCol {
                link: links.and_then(|l| l.get(i)).map(|href| {
                    let name = format!(
                        "{label}: {}, narrow to this range",
                        fmt::count(*v, "commit", "commits")
                    );
                    (href.clone(), name)
                }),
                tip: charttip::part(label, *v, all, unit),
                hx: P(x - (sl - bw) / 2.0),
                x: P(x),
                y: P(TOP_PAD + plot - h),
                h: P(h),
            }
        })
        .collect();
    let keys: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
    Bars {
        f: frame(
            text::bars(name, points, unit),
            links.is_some(),
            grid,
            &keys,
            bw,
        ),
        cols,
    }
}

/// Stacked columns per bucket by the `SERIES` biggest labels (rest folded into `other`),
/// with a legend. Series order is overall frequency, biggest at the bottom.
pub(crate) fn stacked(name: &str, buckets: &[gitterrace_index::LabelBucket]) -> Stacked {
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
    let marks = cols
        .iter()
        .enumerate()
        .map(|(i, col)| {
            let x = (i as f64).mul_add(sl, LEFT) + (sl - bw) / 2.0;
            let parts: Vec<(&str, u64)> = col
                .iter()
                .enumerate()
                .map(|(k, v)| (names.get(k).copied().unwrap_or("other"), *v))
                .collect();
            let mut y = TOP_PAD + plot;
            let segs = col
                .iter()
                .enumerate()
                .filter(|(_, v)| **v > 0)
                .map(|(k, v)| {
                    let h = *v as f64 / top * plot;
                    y -= h;
                    Seg {
                        class: cat(k, names.len()),
                        y: P(y),
                        h: P(h),
                    }
                })
                .collect();
            StackCol {
                tip: charttip::breakdown(&buckets[i].label, &parts, COMMITS),
                hx: P(x - (sl - bw) / 2.0),
                x: P(x),
                segs,
            }
        })
        .collect();
    let keys: Vec<&str> = buckets.iter().map(|b| b.label.as_str()).collect();
    let mut series = names.clone();
    if has_other {
        series.push("other");
    }
    Stacked {
        f: frame(
            text::stacked(name, &keys, &series, &cols),
            false,
            grid,
            &keys,
            bw,
        ),
        cols: marks,
        legend: names
            .iter()
            .enumerate()
            .map(|(k, n)| (cat(k, names.len()), (*n).to_owned()))
            .collect(),
        other: has_other,
    }
}

/// Additions above the axis and deletions below it, one column per bucket.
pub(crate) fn diverging(points: &[(String, u64, u64)], title: &str) -> Diverging {
    let up = points.iter().map(|p| p.1).max().unwrap_or(0);
    let down = points.iter().map(|p| p.2).max().unwrap_or(0);
    // Each side reaches its own maximum on one shared step, so the zero line sits where the
    // data puts it: a period of mostly additions gives the additions most of the height.
    let (step, top_up, top_dn) = split_ticks(up, down, MAX_TICKS);
    let plot = H - TOP_PAD - BOTTOM;
    let px = plot / (top_up + top_dn) as f64;
    let mid = (top_up as f64).mul_add(px, TOP_PAD);
    let mut grid = Vec::new();
    for (sign, top, dir) in [("", top_up, -1.0), ("\u{2212}", top_dn, 1.0)] {
        let mut v = if sign.is_empty() { 0 } else { step };
        while v <= top {
            let y = (v as f64 * px).mul_add(dir, mid);
            grid.push(tick(y, format!("{sign}{}", fmt::thousands(v))));
            v += step;
        }
    }
    let sl = slot(points.len());
    let bw = (sl * 0.8).clamp(1.0, 48.0);
    let cols = points
        .iter()
        .enumerate()
        .map(|(i, (label, a, d))| {
            let x = (i as f64).mul_add(sl, LEFT) + (sl - bw) / 2.0;
            let (ha, hd) = (*a as f64 * px, *d as f64 * px);
            DivCol {
                tip: charttip::breakdown(
                    &format!("{label}, lines changed"),
                    &[("added", *a), ("removed", *d)],
                    charttip::LINES,
                ),
                hx: P(x - (sl - bw) / 2.0),
                x: P(x),
                up_y: P(mid - ha),
                ha: P(ha),
                hd: P(hd),
            }
        })
        .collect();
    let keys: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
    Diverging {
        f: frame(text::diverging(title, points), false, grid, &keys, bw),
        mid: P(mid),
        cols,
    }
}

/// A 0..100 percent line over buckets; `None` values break the line.
pub(crate) fn percent_line(points: &[(String, Option<f64>)]) -> PercentLine {
    let plot = H - TOP_PAD - BOTTOM;
    let grid = [0, 25, 50, 75, 100]
        .into_iter()
        .map(|p| {
            let y = (f64::from(p) / 100.0).mul_add(-plot, TOP_PAD + plot);
            tick(y, format!("{p}%"))
        })
        .collect();
    let sl = slot(points.len());
    // Hover columns first, so they paint under the line.
    let readings = points
        .iter()
        .enumerate()
        .filter_map(|(i, (label, v))| v.map(|v| (i, label, v)))
        .map(|(i, label, v)| {
            let x = (i as f64).mul_add(sl, LEFT);
            LinePoint {
                tip: charttip::rate(label, v, "of changed lines replaced existing code"),
                x: P(x),
                cx: P(x + sl / 2.0),
                cy: P(v.clamp(0.0, 1.0).mul_add(-plot, TOP_PAD + plot)),
            }
        })
        .collect();
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
    let keys: Vec<&str> = points.iter().map(|p| p.0.as_str()).collect();
    PercentLine {
        f: frame(
            text::percent_line("Rework share over time", points),
            false,
            grid,
            &keys,
            0.0,
        ),
        points: readings,
        d,
    }
}

/// Tiny polyline of `values`, 120x22, for table rows.
pub(crate) fn sparkline(values: &[u32], class: &str) -> Spark {
    let max = values.iter().copied().max().unwrap_or(0).max(1);
    let n = values.len().max(2) - 1;
    let mut pts = String::new();
    for (i, v) in values.iter().enumerate() {
        let x = i as f64 * 120.0 / n as f64;
        let y = 20.0 - f64::from(*v) * 18.0 / f64::from(max);
        let _ = write!(pts, "{x:.1},{y:.1} ");
    }
    Spark {
        class: class.to_owned(),
        points: pts.trim_end().to_owned(),
    }
}

/// One dot of [`scatter`].
pub(crate) struct Dot {
    pub(crate) x: u64,
    pub(crate) y: u64,
    /// Radius driver, for example the number of authors.
    pub(crate) size: u64,
    pub(crate) href: String,
    pub(crate) title: String,
    /// What the dot is, for its row in the data table.
    pub(crate) name: String,
}

/// Dots on a linear plane: x along the bottom, y up the side, each dot a link.
pub(crate) fn scatter(dots: &[Dot], x_title: &str) -> Scatter {
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
    let mut xticks = Vec::new();
    let mut v = 0;
    while v <= xtop_n {
        let x = (v as f64 / xtop).mul_add(W - LEFT - 6.0, LEFT);
        xticks.push((P(x), fmt::thousands(v)));
        v += xstep;
    }
    let marks = dots
        .iter()
        .map(|d| ScatterDot {
            href: d.href.clone(),
            title: d.title.clone(),
            cx: P((d.x as f64 / xtop).mul_add(W - LEFT - 6.0, LEFT)),
            cy: P((d.y as f64 / ytop).min(1.0).mul_add(-plot, TOP_PAD + plot)),
            r: P(5.0f64.mul_add((d.size as f64 / smax as f64).sqrt(), 3.0)),
        })
        .collect();
    let named: Vec<(&str, u64, u64, u64)> = dots
        .iter()
        .map(|d| (d.name.as_str(), d.x, d.y, d.size))
        .collect();
    Scatter {
        f: frame(
            text::scatter("Change frequency against size", x_title, &named),
            true,
            grid,
            &[],
            0.0,
        ),
        xticks,
        dots: marks,
        x_title: x_title.to_owned(),
    }
}

/// Weekday by hour grid. Cell intensity is bucketed into five levels.
pub(crate) fn heatmap(cells: &[[u32; 24]; 7], max: u32) -> Heatmap {
    // Sized near the bar charts' width so labels render at the same pixel size.
    const CELL: usize = 38;
    const LEFT: usize = 40;
    const TOP: usize = 20;
    let all: u64 = cells.iter().flatten().map(|v| u64::from(*v)).sum();
    let rows = cells
        .iter()
        .enumerate()
        .map(|(d, row)| HeatRow {
            y: TOP + d * CELL,
            ly: TOP + d * CELL + 22,
            day: DAYS[d],
            cells: row
                .iter()
                .enumerate()
                .map(|(hr, v)| HeatCell {
                    level: if *v == 0 || max == 0 {
                        0
                    } else {
                        (*v * 4).div_ceil(max).min(4)
                    },
                    x: LEFT + hr * CELL,
                    tip: charttip::part(
                        &format!("{} {hr:02}:00", DAYS[d]),
                        u64::from(*v),
                        all,
                        COMMITS,
                    ),
                })
                .collect(),
        })
        .collect();
    Heatmap {
        fig: text::heatmap("Commits by weekday and hour", cells),
        w: LEFT + 24 * CELL,
        h: TOP + 7 * CELL,
        side: CELL - 4,
        hours: (0..24)
            .step_by(3)
            .map(|hour| (LEFT + hour * CELL, format!("{hour:02}")))
            .collect(),
        rows,
    }
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
