//! Languages page: lines of code per language at every first-parent commit, a brush that
//! selects a commit range, and the range card the server renders from `?from&to`.

#![allow(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    clippy::needless_range_loop
)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use askama::Template;
use axum::extract::{Query, State};
use axum::response::Html;
use gitterrace_index::{CommitRecord, Insights};
use serde::Deserialize;

use crate::{
    charts::nice_step,
    error::Result,
    fmt::{self, esc},
    routes::{Shared, with_records},
    views::{self, Chrome, Nav},
};

const TOP: usize = 7;
const W: f64 = 960.0;
const H: f64 = 280.0;
const LEFT: f64 = 72.0;
const RIGHT: f64 = 8.0;
const TOP_PAD: f64 = 10.0;
const BOTTOM: f64 = 26.0;
/// Points kept in the inline JSON and the area paths.
const SAMPLES: usize = 700;
const COMMIT_LIST: usize = 500;

/// Lines per language after each commit of the first-parent chain, oldest first.
pub(crate) struct Chain {
    ids: Vec<String>,
    subjects: Vec<String>,
    times: Vec<i64>,
    offsets: Vec<i32>,
    adds: Vec<u64>,
    dels: Vec<u64>,
    /// Top languages, largest first, then `Other` when folded.
    langs: Vec<String>,
    /// `lines[commit][language]`.
    lines: Vec<Vec<i64>>,
}

impl Chain {
    pub(crate) fn build(
        records: &[CommitRecord],
        ins: &Insights<'_>,
        subjects: &HashMap<String, String>,
    ) -> Self {
        let by_id: HashMap<&str, usize> = records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.as_str(), i))
            .collect();
        let mut chain = Vec::new();
        let mut at = records.len().checked_sub(1);
        while let Some(i) = at {
            chain.push(i);
            at = records[i]
                .parents
                .first()
                .and_then(|p| by_id.get(p.as_str()).copied());
        }
        chain.reverse();

        let mut names: Vec<String> = Vec::new();
        let mut slot: HashMap<String, usize> = HashMap::new();
        let mut lang_of: HashMap<&str, usize> = HashMap::new();
        let mut running: Vec<i64> = Vec::new();
        let mut rows: Vec<Vec<i64>> = Vec::with_capacity(chain.len());
        let (mut adds, mut dels) = (Vec::new(), Vec::new());
        for &i in &chain {
            let (mut a, mut d) = (0u64, 0u64);
            for f in &records[i].files {
                if ins.skips(&f.path) {
                    continue;
                }
                let s = *lang_of.entry(&*f.path).or_insert_with(|| {
                    let l = gitterrace_index::language(&f.path);
                    *slot.entry(l.clone()).or_insert_with(|| {
                        names.push(l);
                        running.push(0);
                        names.len() - 1
                    })
                });
                running[s] += i64::from(f.additions) - i64::from(f.deletions);
                a += u64::from(f.additions);
                d += u64::from(f.deletions);
            }
            adds.push(a);
            dels.push(d);
            rows.push(running.clone());
        }
        let last = rows.last().cloned().unwrap_or_default();
        let mut order: Vec<usize> = (0..names.len()).filter(|s| last[*s] > 0).collect();
        order.sort_by(|a, b| {
            last[*b]
                .cmp(&last[*a])
                .then_with(|| names[*a].cmp(&names[*b]))
        });
        let keep: Vec<usize> = if order.len() <= TOP + 1 {
            order
        } else {
            order[..TOP].to_vec()
        };
        let folded = names.len() > keep.len();
        let mut langs: Vec<String> = keep.iter().map(|s| names[*s].clone()).collect();
        if folded {
            langs.push("Other".to_owned());
        }
        let lines = rows
            .iter()
            .map(|row| {
                let mut v: Vec<i64> = keep
                    .iter()
                    .map(|s| row.get(*s).copied().unwrap_or(0))
                    .collect();
                if folded {
                    let kept: i64 = v.iter().sum();
                    let all: i64 = row.iter().sum();
                    v.push(all - kept);
                }
                v
            })
            .collect();
        Self {
            ids: chain.iter().map(|i| records[*i].id.clone()).collect(),
            subjects: chain
                .iter()
                .map(|i| subjects.get(&records[*i].id).cloned().unwrap_or_default())
                .collect(),
            times: chain.iter().map(|i| records[*i].time).collect(),
            offsets: chain.iter().map(|i| records[*i].offset).collect(),
            adds,
            dels,
            langs,
            lines,
        }
    }

    fn len(&self) -> usize {
        self.ids.len()
    }

    /// Lines per language before commit `i` (all zero before the first).
    fn before(&self, i: usize) -> Vec<i64> {
        if i == 0 {
            vec![0; self.langs.len()]
        } else {
            self.lines[i - 1].clone()
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LocQuery {
    x: Option<String>,
    scale: Option<String>,
    from: Option<usize>,
    to: Option<usize>,
}

/// View state carried by every link on the page.
#[derive(Clone, Copy)]
struct View {
    time: bool,
    share: bool,
    range: Option<(usize, usize)>,
}

impl View {
    fn parse(q: &LocQuery, n: usize) -> Self {
        let range = match (q.from, q.to) {
            (None, None) => None,
            (f, t) => {
                let last = n.saturating_sub(1);
                let to = t.unwrap_or(last).min(last);
                let from = f.unwrap_or(0).min(to);
                Some((from, to))
            }
        };
        Self {
            time: q.x.as_deref() == Some("time"),
            share: q.scale.as_deref() == Some("share"),
            range,
        }
    }

    fn href(self, f: impl FnOnce(&mut Self)) -> String {
        let mut v = self;
        f(&mut v);
        let mut q: Vec<String> = Vec::new();
        if v.time {
            q.push("x=time".to_owned());
        }
        if v.share {
            q.push("scale=share".to_owned());
        }
        if let Some((a, b)) = v.range {
            q.push(format!("from={a}"));
            q.push(format!("to={b}"));
        }
        if q.is_empty() {
            fmt::href("/stats/languages")
        } else {
            fmt::href(&format!("/stats/languages?{}", q.join("&")))
        }
    }
}

fn seg(label: &str, items: &[(&str, bool, String)]) -> String {
    let mut s =
        format!(r#"<div class="seg" role="group" aria-label="{label}"><span>{label}</span>"#);
    for (text, on, href) in items {
        let _ = write!(
            s,
            r#"<a href="{href}"{}>{text}</a>"#,
            if *on {
                r#" class="on" aria-current="true""#
            } else {
                ""
            }
        );
    }
    s.push_str("</div>");
    s
}

fn signed(n: i64) -> String {
    if n >= 0 {
        format!("+{}", fmt::thousands(n))
    } else {
        format!("\u{2212}{}", fmt::thousands(-n))
    }
}

fn pct(f: f64) -> String {
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
fn sample(n: usize, keep: &[usize]) -> Vec<usize> {
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

fn area_chart(ch: &Chain, view: View) -> String {
    let n = ch.len();
    if n == 0 {
        return r#"<p class="empty">No history yet.</p>"#.to_owned();
    }
    let keep: Vec<usize> = view.range.map_or(vec![], |(a, b)| vec![a, b]);
    let pts = sample(n, &keep);
    let (t0, t1) = (ch.times[0], ch.times[n - 1].max(ch.times[0] + 1));
    let pw = W - LEFT - RIGHT;
    let ph = H - TOP_PAD - BOTTOM;
    let mut xs: Vec<f64> = Vec::new();
    for &i in &pts {
        let frac = if view.time {
            (ch.times[i] - t0) as f64 / (t1 - t0) as f64
        } else {
            i as f64 / (n - 1).max(1) as f64
        };
        let x = LEFT + frac.clamp(0.0, 1.0) * pw;
        xs.push(xs.last().map_or(x, |p: &f64| p.max(x)));
    }
    let totals: Vec<i64> = pts
        .iter()
        .map(|i| ch.lines[*i].iter().map(|v| (*v).max(0)).sum())
        .collect();
    let max = totals.iter().copied().max().unwrap_or(0).max(1);
    let (ymax, step) = if view.share {
        (100.0, 25u64)
    } else {
        let step = nice_step(max as u64);
        let top = step * (max as u64).div_ceil(step).max(1);
        (top as f64, step)
    };
    let yat = |v: f64| TOP_PAD + ph - v / ymax * ph;

    let mut svg = format!(
        r#"<svg class="chart loc" id="loc" viewBox="0 0 {W:.0} {H:.0}" data-left="{LEFT}" data-right="{:.1}" role="img" aria-label="Lines of code by language over history">"#,
        W - RIGHT
    );
    let mut v = 0u64;
    while (v as f64) <= ymax {
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
    let k = ch.langs.len();
    let mut base = vec![0.0f64; pts.len()];
    for lang in 0..k {
        let mut top = Vec::with_capacity(pts.len());
        for (j, &i) in pts.iter().enumerate() {
            let raw = ch.lines[i][lang].max(0) as f64;
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
            esc(&ch.langs[lang])
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
            fmt::day(ch.times[pts[j]])
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

    let data = serde_json::json!({
        "n": n,
        "share": view.share,
        "names": ch.langs,
        "i": pts,
        "x": xs.iter().map(|x| (x * 10.0).round() / 10.0).collect::<Vec<_>>(),
        "d": pts.iter().map(|i| fmt::date(ch.times[*i], ch.offsets[*i])).collect::<Vec<_>>(),
        "s": pts.iter().map(|i| ch.subjects[*i].clone()).collect::<Vec<_>>(),
        "l": pts.iter().map(|i| ch.lines[*i].clone()).collect::<Vec<_>>(),
        "view": view.href(|v| v.range = None),
    });
    let json = data.to_string().replace('<', "\\u003c");
    format!(
        r#"<div class="plot">{svg}<div class="loc-tip" hidden></div></div><script type="application/json" id="loc-data">{json}</script>"#
    )
}

fn legend(ch: &Chain) -> String {
    let k = ch.langs.len();
    let mut s = String::from(r#"<div class="legend">"#);
    for (i, l) in ch.langs.iter().enumerate() {
        let _ = write!(s, r#"<span><i class="{}"></i>{}</span>"#, cat(i, k), esc(l));
    }
    s.push_str("</div>");
    s
}

fn range_card(ch: &Chain, view: View) -> String {
    let n = ch.len();
    if n == 0 {
        return String::new();
    }
    let (from, to) = view.range.unwrap_or((0, n - 1));
    let base = ch.before(from);
    let end = &ch.lines[to];
    let count = to - from + 1;
    let added: u64 = ch.adds[from..=to].iter().sum();
    let removed: u64 = ch.dels[from..=to].iter().sum();
    let net = added as i64 - removed as i64;
    let k = ch.langs.len();
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
        format!(
            r#"<a href="{href}"{}>{label}</a>"#,
            if on { r#" class="on""# } else { "" }
        )
    };
    let mut s = String::from(r#"<section class="card pad range" id="range">"#);
    let _ = write!(
        s,
        r#"<div class="chart-head"><div><h2>{}</h2><p class="hint">{} to {}. Drag across the chart to pick another range.</p></div><div class="seg">{}{}{}</div></div>"#,
        if view.range.is_some() {
            "Selected range"
        } else {
            "Whole history"
        },
        fmt::day(ch.times[from]),
        fmt::day(ch.times[to]),
        preset("Last 10", Some(10)),
        preset("Last 25", Some(25)),
        preset("Whole history", None),
    );
    let _ = write!(
        s,
        r#"<div class="kpis"><div><b>{}</b><span>commits</span></div><div><b class="add">+{}</b><span>lines added</span></div><div><b class="del">&minus;{}</b><span>lines removed</span></div><div><b class="{}">{}</b><span>net</span></div></div>"#,
        fmt::thousands(count),
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
            r#"<tr><td><i class="dot {}"></i>{}</td><td class="r">{}</td><td class="r">{} <span class="muted">{}</span></td><td class="r"><b class="{}">{}</b></td><td class="dcol"><span class="dbar"><b class="{}" style="width:{w}%"></b></span></td></tr>"#,
            cat(i, k),
            esc(&ch.langs[i]),
            fmt::thousands(base[i].max(0)),
            fmt::thousands(end[i].max(0)),
            pct(end[i].max(0) as f64 / end_total as f64),
            if d >= 0 { "add" } else { "del" },
            signed(d),
            if d >= 0 { "up" } else { "dn" },
        );
    }
    s.push_str("</table></div>");
    let shown = count.min(COMMIT_LIST);
    let _ = write!(
        s,
        r#"<details class="rlist"><summary>{} commits in this range</summary><ul>"#,
        fmt::thousands(count)
    );
    for i in (to + 1 - shown..=to).rev() {
        let _ = write!(
            s,
            r#"<li>{} <span class="subj" title="{}">{}</span> <span class="muted">{}</span></li>"#,
            fmt::sha_link(&ch.ids[i]),
            esc(&ch.subjects[i]),
            esc(&ch.subjects[i]),
            fmt::day(ch.times[i])
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
    let k = ch.langs.len();
    let last = &ch.lines[n - 1];
    let back = &ch.lines[n.saturating_sub(11)];
    let total: i64 = last.iter().map(|v| (*v).max(0)).sum::<i64>().max(1);
    let mut bar = String::from(
        r#"<div class="sharebar" role="img" aria-label="Share of lines by language">"#,
    );
    for i in 0..k {
        let f = last[i].max(0) as f64 / total as f64;
        if f > 0.0 {
            let _ = write!(
                bar,
                r#"<b class="{}" style="width:{:.3}%" data-tip="{} {}"></b>"#,
                cat(i, k),
                f * 100.0,
                esc(&ch.langs[i]),
                pct(f)
            );
        }
    }
    bar.push_str("</div>");
    let mut s = String::from(
        r#"<section class="card pad"><div class="chart-head"><div><h2>Languages now</h2><p class="hint">Share of all lines at the newest commit, trend over the whole history, and change over the last 10 commits.</p></div></div>"#,
    );
    s.push_str(&bar);
    s.push_str(r#"<div class="scroll"><table class="entries"><tr class="th"><td>Language</td><td class="r">Lines</td><td class="r">Share</td><td>Trend</td><td class="r">Last 10 commits</td></tr>"#);
    let step = (n / 40).max(1);
    for i in 0..k {
        let mut maxv = 1i64;
        let mut series: Vec<i64> = (0..n)
            .step_by(step)
            .map(|c| ch.lines[c][i].max(0))
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
                    j as f64 / m as f64 * 88.0 + 1.0,
                    19.0 - *v as f64 / maxv as f64 * 18.0
                )
            })
            .collect();
        let d = last[i] - back[i];
        let _ = write!(
            s,
            r#"<tr><td><i class="dot {}"></i>{}</td><td class="r">{}</td><td class="r">{}</td><td><svg class="spark {}" viewBox="0 0 90 20" width="90" height="20" aria-hidden="true"><polyline points="{}"/></svg></td><td class="r"><b class="{}">{}</b></td></tr>"#,
            cat(i, k),
            esc(&ch.langs[i]),
            fmt::thousands(last[i].max(0)),
            pct(last[i].max(0) as f64 / total as f64),
            cat_s(i, k),
            pts.join(" "),
            if d >= 0 { "add" } else { "del" },
            if d == 0 { "0".to_owned() } else { signed(d) },
        );
    }
    s.push_str("</table></div></section>");
    s
}

fn body(ch: &Chain, view: View) -> String {
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
    let total: i64 = ch.lines.last().map_or(0, |l| l.iter().sum());
    format!(
        r#"{filters}<section class="card pad"><div class="chart-head"><div><h2>Lines of code by language</h2><p class="hint">Exact line count at every commit on the first-parent history, {} lines today. Drag across the chart to select a range.</p></div>{}</div>{}</section>{}{}"#,
        fmt::thousands(total.max(0)),
        legend(ch),
        area_chart(ch, view),
        range_card(ch, view),
        languages_now(ch),
    )
}

#[derive(Template)]
#[template(path = "languages.html")]
struct LanguagesPage {
    c: Chrome,
    tabs: Vec<Nav>,
    body: String,
}

pub(crate) async fn page(State(app): Shared, Query(q): Query<LocQuery>) -> Result<Html<String>> {
    let a2 = Arc::clone(&app);
    with_records(&app, "l", move |a, r, c| {
        let head = a.repo.resolve(&crate::fmt::selected()).unwrap_or_default();
        let ch = a2.chain(&head, r);
        let view = View::parse(&q, ch.len());
        LanguagesPage {
            c,
            tabs: views::stats_tabs("l"),
            body: body(&ch, view),
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::{LocQuery, View, sample};

    #[test]
    fn range_is_clamped_and_ordered() {
        let q = LocQuery {
            from: Some(50),
            to: Some(7),
            ..LocQuery::default()
        };
        assert_eq!(View::parse(&q, 100).range, Some((7, 7)));
        let q = LocQuery {
            from: Some(3),
            to: Some(999),
            ..LocQuery::default()
        };
        assert_eq!(View::parse(&q, 100).range, Some((3, 99)));
        assert_eq!(View::parse(&LocQuery::default(), 100).range, None);
    }

    #[test]
    fn sampling_keeps_ends_and_selection() {
        let s = sample(10_000, &[1234, 5678]);
        assert!(s.len() <= 702);
        assert_eq!((s[0], *s.last().unwrap_or(&0)), (0, 9_999));
        assert!(s.contains(&1234) && s.contains(&5678));
        assert_eq!(sample(5, &[]), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn links_keep_state() {
        let v = View {
            time: true,
            share: false,
            range: Some((1, 2)),
        };
        assert_eq!(
            v.href(|v| v.share = true),
            "/stats/languages?x=time&scale=share&from=1&to=2"
        );
    }
}
