//! The chart, range card and language table as HTML.

#![expect(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    clippy::needless_range_loop,
    reason = "geometry and colour math reads best with x, y, w, h; one linear sequence, splitting it hides the flow; the index is used for several parallel arrays"
)]

use std::fmt::Write as _;

use crate::charts::nice_step;
use crate::fmt::{self, esc};
use crate::languages::chain::Chain;
use crate::languages::{
    BOTTOM, COMMIT_LIST, H, LEFT, RIGHT, SAMPLES, Subjects, TOP, TOP_PAD, View, W,
};
use crate::seg::{Choice, Seg};

fn seg(label: &str, items: &[(&str, bool, String)]) -> String {
    Seg::links(
        label,
        items
            .iter()
            .map(|(text, on, href)| Choice::new(*text, href.clone(), *on))
            .collect(),
    )
    .to_string()
}

fn signed(n: i64) -> String {
    if n >= 0 {
        format!("+{}", fmt::thousands(n))
    } else {
        format!("\u{2212}{}", fmt::thousands(-n))
    }
}

fn share_label(f: f64) -> String {
    if f > 0.0 && f < 0.001 {
        "<0.1%".to_owned()
    } else {
        format!("{:.1}%", f * 100.0)
    }
}

fn cat(k: usize, n: usize) -> String {
    // The last entry is "Other" whenever the chain folded languages.
    crate::charts::cat(k, n.min(TOP))
}

fn cat_s(k: usize, n: usize) -> String {
    cat(k, n).replacen('c', "s", 1)
}

/// Sampled commit indices: evenly spread, always with first, last and `keep`.
pub(super) fn sample(n: usize, keep: &[usize]) -> Vec<usize> {
    let mut v: Vec<usize> = if n <= SAMPLES {
        (0..n).collect()
    } else {
        (0..SAMPLES).map(|k| k * (n - 1) / (SAMPLES - 1)).collect()
    };
    v.extend(keep.iter().filter(|k| **k < n));
    v.sort_unstable();
    v.dedup();
    v
}

fn area_chart(ch: &Chain, view: View, subj: &Subjects) -> String {
    let n = ch.len();
    if n == 0 {
        return r#"<p class="empty">No history yet.</p>"#.to_owned();
    }
    let keep: Vec<usize> = view.range.map_or(vec![], |(a, b)| vec![a, b]);
    let pts = sample(n, &keep);
    let (t0, t1) = (ch.time(0), ch.time(n - 1).max(ch.time(0) + 1));
    let pw = W - LEFT - RIGHT;
    let ph = H - TOP_PAD - BOTTOM;
    let mut xs: Vec<f64> = Vec::new();
    for &i in &pts {
        let frac = if view.time {
            (ch.time(i) - t0) as f64 / (t1 - t0) as f64
        } else {
            i as f64 / (n - 1).max(1) as f64
        };
        let x = frac.clamp(0.0, 1.0).mul_add(pw, LEFT);
        xs.push(xs.last().map_or(x, |p: &f64| p.max(x)));
    }
    let rows: Vec<Vec<i64>> = pts.iter().map(|i| ch.lines(*i)).collect();
    let totals: Vec<i64> = pts
        .iter()
        .map(|i| ch.lines(*i).iter().map(|v| (*v).max(0)).sum())
        .collect();
    let max = totals.iter().copied().max().unwrap_or(0).max(1);
    let (ymax_n, step) = if view.share {
        (100u64, 25u64)
    } else {
        let step = nice_step(max as u64);
        (step * (max as u64).div_ceil(step).max(1), step)
    };
    let ymax = ymax_n as f64;
    let yat = |v: f64| (v / ymax).mul_add(-ph, TOP_PAD + ph);

    let mut svg = format!(
        r#"<svg class="chart loc" id="loc" viewBox="0 0 {W:.0} {H:.0}" data-left="{LEFT}" data-right="{:.1}" role="img" aria-label="Lines of code by language over history">"#,
        W - RIGHT
    );
    let mut v = 0u64;
    while v <= ymax_n {
        let y = yat(v as f64);
        let label = if view.share {
            format!("{v}%")
        } else {
            fmt::thousands(v)
        };
        let _ = write!(
            svg,
            r#"<line class="grid" x1="{LEFT}" x2="{:.1}" y1="{y:.1}" y2="{y:.1}"/><text class="ax" x="{:.1}" y="{:.1}" text-anchor="end">{label}</text>"#,
            W - RIGHT,
            LEFT - 8.0,
            y + 3.5
        );
        v += step;
    }
    // Stack from the bottom: language 0 sits on the axis.
    let k = ch.langs().len();
    let mut base = vec![0.0f64; pts.len()];
    for lang in 0..k {
        let mut top = Vec::with_capacity(pts.len());
        for j in 0..pts.len() {
            let raw = rows[j][lang].max(0) as f64;
            let val = if view.share {
                if totals[j] > 0 {
                    raw / totals[j] as f64 * 100.0
                } else {
                    0.0
                }
            } else {
                raw
            };
            top.push(base[j] + val);
        }
        let mut d = String::new();
        for (j, x) in xs.iter().enumerate() {
            let _ = write!(
                d,
                "{}{x:.1},{:.1}",
                if j == 0 { "M" } else { "L" },
                yat(top[j])
            );
        }
        for (j, x) in xs.iter().enumerate().rev() {
            let _ = write!(d, "L{x:.1},{:.1}", yat(base[j]));
        }
        let _ = write!(
            svg,
            r#"<path class="{}" d="{d}Z" data-tip="{}"/>"#,
            cat(lang, k),
            esc(&ch.langs()[lang])
        );
        base = top;
    }
    // Date labels under the plot.
    for s in 0..=4u8 {
        let want = LEFT + pw * f64::from(s) / 4.0;
        let j = xs
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 - want).abs().total_cmp(&(b.1 - want).abs()))
            .map_or(0, |m| m.0);
        let anchor = match s {
            0 => "start",
            4 => "end",
            _ => "middle",
        };
        let _ = write!(
            svg,
            r#"<text class="ax" x="{:.1}" y="{:.1}" text-anchor="{anchor}">{}</text>"#,
            xs[j],
            H - 8.0,
            fmt::day(ch.time(pts[j]))
        );
    }
    if let Some((a, b)) = view.range {
        let ja = pts.iter().position(|p| *p == a).unwrap_or(0);
        let jb = pts.iter().position(|p| *p == b).unwrap_or(pts.len() - 1);
        let _ = write!(
            svg,
            r#"<rect class="sel" x="{:.1}" y="{TOP_PAD}" width="{:.1}" height="{ph:.1}"/>"#,
            xs[ja],
            (xs[jb] - xs[ja]).max(2.0)
        );
    } else {
        let _ = write!(
            svg,
            r#"<rect class="sel" x="0" y="{TOP_PAD}" width="0" height="{ph:.1}"/>"#
        );
    }
    let _ = write!(
        svg,
        r#"<line class="xh" x1="0" x2="0" y1="{TOP_PAD}" y2="{:.1}"/></svg>"#,
        TOP_PAD + ph
    );

    let data = PlotData {
        d: pts
            .iter()
            .map(|i| fmt::date(ch.time(*i), ch.offset(*i)))
            .collect(),
        i: &pts,
        l: rows,
        n,
        names: ch.langs(),
        s: pts.iter().map(|i| subj.of(*i)).collect(),
        share: view.share,
        view: view.href(|v| v.range = None),
        x: xs.iter().map(|x| (x * 10.0).round() / 10.0).collect(),
    };
    // Strings and numbers only: serialization cannot fail.
    let json = serde_json::to_string(&data)
        .unwrap_or_default()
        .replace('<', "\\u003c");
    format!(
        r#"<div class="plot">{svg}<div class="loc-tip" hidden></div></div><script type="application/json" id="loc-data">{json}</script>"#
    )
}

/// What `loc.js` reads from the page: per sampled point its date, subject and per-language
/// lines, and the x position of each.
// Fields are declared in alphabetical order: this was a serde_json map, which sorts keys.
#[derive(serde::Serialize)]
struct PlotData<'a> {
    d: Vec<String>,
    i: &'a [usize],
    l: Vec<Vec<i64>>,
    n: usize,
    names: &'a [String],
    s: Vec<String>,
    share: bool,
    view: String,
    x: Vec<f64>,
}

fn legend(ch: &Chain) -> String {
    let k = ch.langs().len();
    let mut s = String::from(r#"<div class="legend">"#);
    for (i, l) in ch.langs().iter().enumerate() {
        let _ = write!(s, r#"<span><i class="{}"></i>{}</span>"#, cat(i, k), esc(l));
    }
    s.push_str("</div>");
    s
}

fn range_card(ch: &Chain, view: View, subj: &Subjects) -> String {
    let n = ch.len();
    if n == 0 {
        return String::new();
    }
    let (from, to) = view.range.unwrap_or((0, n - 1));
    let base = ch.before(from);
    let end = &ch.lines(to);
    let count = to - from + 1;
    let (added, removed) = (ch.added, ch.removed);
    let net = added as i64 - removed as i64;
    let k = ch.langs().len();
    let deltas: Vec<i64> = (0..k).map(|i| end[i] - base[i]).collect();
    let big = deltas
        .iter()
        .map(|d| d.unsigned_abs())
        .max()
        .unwrap_or(1)
        .max(1);
    let end_total: i64 = end.iter().sum::<i64>().max(1);

    let preset = |label: &str, size: Option<usize>| {
        let href = view.href(|v| {
            v.range = size.map(|s| (n.saturating_sub(s), n - 1));
        });
        let on = match (size, view.range) {
            (None, None) => true,
            (Some(s), Some((a, b))) => b == n - 1 && a == n.saturating_sub(s),
            _ => false,
        };
        Choice::new(label, href, on)
    };
    let presets = Seg::links(
        "Range",
        vec![
            preset("Last 10", Some(10)),
            preset("Last 25", Some(25)),
            preset("Whole history", None),
        ],
    )
    .no_caption();
    let mut s = String::from(r#"<section class="card pad range" id="range">"#);
    let _ = write!(
        s,
        r#"<div class="chart-head"><div><h2>{}</h2><p class="hint">{} to {}. Drag across the chart to pick another range.</p></div>{}</div>"#,
        if view.range.is_some() {
            "Selected range"
        } else {
            "Whole history"
        },
        fmt::day(ch.time(from)),
        fmt::day(ch.time(to)),
        presets,
    );
    let _ = write!(
        s,
        r#"<div class="kpis"><div><b>{}</b><span>{}</span></div><div><b class="add">+{}</b><span>lines added</span></div><div><b class="del">&minus;{}</b><span>lines removed</span></div><div><b class="{}">{}</b><span>net</span></div></div>"#,
        fmt::thousands(count),
        fmt::noun(count, "commit", "commits"),
        fmt::thousands(added),
        fmt::thousands(removed),
        if net >= 0 { "add" } else { "del" },
        signed(net)
    );
    s.push_str(r#"<div class="scroll"><table class="entries"><tr class="th"><td>Language</td><td class="r">Before</td><td class="r">After</td><td class="r">Change</td><td class="dcol"></td></tr>"#);
    for i in 0..k {
        let d = deltas[i];
        let w = (d.unsigned_abs() * 100 / big).max(u64::from(d != 0));
        let _ = write!(
            s,
            r#"<tr><td><i class="dot {}"></i>{}</td><td class="r">{}</td><td class="r">{} <span class="muted">{}</span></td><td class="r"><b class="{}">{}</b></td><td class="dcol"><span class="dbar"><b class="{} w{w}"></b></span></td></tr>"#,
            cat(i, k),
            esc(&ch.langs()[i]),
            fmt::thousands(base[i].max(0)),
            fmt::thousands(end[i].max(0)),
            share_label(end[i].max(0) as f64 / end_total as f64),
            if d >= 0 { "add" } else { "del" },
            signed(d),
            if d >= 0 { "up" } else { "dn" },
        );
    }
    s.push_str("</table></div>");
    let shown = count.min(COMMIT_LIST);
    let _ = write!(
        s,
        r#"<details class="rlist"><summary>{} in this range</summary><ul>"#,
        fmt::count(count, "commit", "commits")
    );
    for i in (to + 1 - shown..=to).rev() {
        let _ = write!(
            s,
            r#"<li>{} <span class="subj">{}</span> <span class="muted">{}</span></li>"#,
            fmt::sha_link(ch.id(i)),
            esc(&subj.of(i)),
            fmt::day(ch.time(i))
        );
    }
    s.push_str("</ul>");
    if shown < count {
        let _ = write!(
            s,
            r#"<p class="foot">Showing the newest {shown}. Narrow the range to see the rest.</p>"#
        );
    }
    s.push_str("</details></section>");
    s
}

fn languages_now(ch: &Chain) -> String {
    let n = ch.len();
    if n == 0 {
        return String::new();
    }
    let k = ch.langs().len();
    let last = &ch.lines(n - 1);
    let back = ch.before(n.saturating_sub(10));
    let total: i64 = last.iter().map(|v| (*v).max(0)).sum::<i64>().max(1);
    let mut bar = String::from(
        r#"<div class="sharebar" role="img" aria-label="Share of lines by language">"#,
    );
    for i in 0..k {
        let f = last[i].max(0) as f64 / total as f64;
        if f > 0.0 {
            let _ = write!(
                bar,
                r#"<b class="{} w{}" data-tip="{} {}"></b>"#,
                cat(i, k),
                (f * 100.0).round().clamp(1.0, 100.0) as u32,
                esc(&ch.langs()[i]),
                share_label(f)
            );
        }
    }
    bar.push_str("</div>");
    let mut s = String::from(
        r#"<section class="card pad"><div class="chart-head"><div><h2>Languages now</h2><p class="hint">Share of all lines at the newest commit, trend over the whole history, and change over the last 10 commits.</p></div></div>"#,
    );
    s.push_str(&bar);
    let window = if n <= 10 {
        format!("All {}", fmt::count(n, "commit", "commits"))
    } else {
        "Last 10 commits".to_owned()
    };
    let _ = write!(
        s,
        r#"<div class="scroll"><table class="entries"><tr class="th"><td>Language</td><td class="r">Lines</td><td class="r">Share</td><td>Trend</td><td class="r">{window}</td></tr>"#
    );
    let step = (n / 40).max(1);
    for i in 0..k {
        let mut maxv = 1i64;
        let mut series: Vec<i64> = (0..n)
            .step_by(step)
            .map(|c| ch.lines(c)[i].max(0))
            .collect();
        series.push(last[i].max(0));
        for v in &series {
            maxv = maxv.max(*v);
        }
        let m = series.len().saturating_sub(1).max(1);
        let pts: Vec<String> = series
            .iter()
            .enumerate()
            .map(|(j, v)| {
                format!(
                    "{:.1},{:.1}",
                    (j as f64 / m as f64).mul_add(88.0, 1.0),
                    (*v as f64 / maxv as f64).mul_add(-18.0, 19.0)
                )
            })
            .collect();
        let d = last[i] - back[i];
        let _ = write!(
            s,
            r#"<tr><td><i class="dot {}"></i>{}</td><td class="r">{}</td><td class="r">{}</td><td><svg class="spark {}" viewBox="0 0 90 20" width="90" height="20" aria-hidden="true"><polyline points="{}"/></svg></td><td class="r"><b class="{}">{}</b></td></tr>"#,
            cat(i, k),
            esc(&ch.langs()[i]),
            fmt::thousands(last[i].max(0)),
            share_label(last[i].max(0) as f64 / total as f64),
            cat_s(i, k),
            pts.join(" "),
            if d >= 0 { "add" } else { "del" },
            if d == 0 { "0".to_owned() } else { signed(d) },
        );
    }
    s.push_str("</table></div></section>");
    s
}

pub(super) fn body(ch: &Chain, view: View, subj: &Subjects) -> String {
    if ch.len() == 0 {
        return include_str!("../../templates/_empty.html").to_owned();
    }
    let filters = format!(
        r#"<div class="filters">{}{}</div>"#,
        seg(
            "X axis",
            &[
                ("Per commit", !view.time, view.href(|v| v.time = false)),
                ("Over time", view.time, view.href(|v| v.time = true)),
            ]
        ),
        seg(
            "Scale",
            &[
                ("Lines", !view.share, view.href(|v| v.share = false)),
                ("Share", view.share, view.href(|v| v.share = true)),
            ]
        ),
    );
    let total: i64 = ch.lines(ch.len() - 1).iter().sum();
    format!(
        r#"{filters}<section class="card pad"><div class="chart-head"><div><h2>Lines of code by language</h2><p class="hint">Exact line count at every commit on the first-parent history, {} today. Drag across the chart to select a range.</p></div>{}</div>{}</section>{}{}"#,
        fmt::count(total.max(0), "line", "lines"),
        legend(ch),
        area_chart(ch, view, subj),
        range_card(ch, view, subj),
        languages_now(ch),
    )
}
