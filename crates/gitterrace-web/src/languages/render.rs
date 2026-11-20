//! The chart, range card and language table as typed data for `languages_body.html`.

#![expect(clippy::too_many_lines, reason = "one linear sequence, splitting it hides the flow")]

use std::fmt::Write as _;

use crate::charts::view::{Fig, P, Row, Table, Tick};
use crate::charts::{self, ticks};
use crate::charttip;
use crate::empty::Empty;
use crate::fmt;
use crate::insights::Kpi;
use crate::languages::chain::Chain;
use crate::languages::view::{
    Area, Band, Body, DateLabel, Now, NowRow, RangeCard, RangeCommit, RangeRow, SharePart, Shown,
};
use crate::languages::{
    BOTTOM, COMMIT_LIST, H, Held, LEFT, RIGHT, SAMPLES, Subjects, TOP, TOP_PAD, View, W,
};
use crate::seg::{Choice, Seg};
use gitterrace_index::Period;
use gitterrace_types::StringEnum;

fn signed(n: i64) -> String {
    if n >= 0 {
        format!("+{}", fmt::thousands(n))
    } else {
        format!("\u{2212}{}", fmt::thousands(-n))
    }
}

fn share_label(f: f64) -> String {
    if f > 0.0 && f < 0.001 { "<0.1%".to_owned() } else { format!("{:.1}%", f * 100.0) }
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

const AREA_NAME: &str = "Lines of code by language over history";

fn area_chart(ch: &Chain, view: View, subj: &Subjects, lead: &str) -> (Area, String) {
    let n = ch.len();
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
    let totals: Vec<i64> =
        pts.iter().map(|i| ch.lines(*i).iter().map(|v| (*v).max(0)).sum()).collect();
    let max = totals.iter().copied().max().unwrap_or(0).max(1);
    let (ymax_n, step) = if view.share {
        (100u64, 25u64)
    } else {
        let (step, top) = ticks(max.max(0) as u64, 5);
        (top, step)
    };
    let ymax = ymax_n as f64;
    let yat = |v: f64| (v / ymax).mul_add(-ph, TOP_PAD + ph);

    let mut grid = Vec::new();
    let mut v = 0u64;
    while v <= ymax_n {
        let y = yat(v as f64);
        grid.push(Tick {
            y: P(y),
            ty: P(y + 3.5),
            label: if view.share { format!("{v}%") } else { fmt::thousands(v) },
        });
        v += step;
    }
    // Stack from the bottom: language 0 sits on the axis.
    let k = ch.langs().len();
    let mut base = vec![0.0f64; pts.len()];
    let mut bands = Vec::with_capacity(k);
    for lang in 0..k {
        let mut top = Vec::with_capacity(pts.len());
        for j in 0..pts.len() {
            let raw = rows[j][lang].max(0) as f64;
            let val = if view.share {
                if totals[j] > 0 { raw / totals[j] as f64 * 100.0 } else { 0.0 }
            } else {
                raw
            };
            top.push(base[j] + val);
        }
        let mut d = String::new();
        for (j, x) in xs.iter().enumerate() {
            let _ = write!(d, "{}{x:.1},{:.1}", if j == 0 { "M" } else { "L" }, yat(top[j]));
        }
        for (j, x) in xs.iter().enumerate().rev() {
            let _ = write!(d, "L{x:.1},{:.1}", yat(base[j]));
        }
        // Without the script a band's tip reads the newest shown point; the script shows the
        // hovered date's full breakdown instead.
        let last = pts.len() - 1;
        bands.push(Band {
            class: cat(lang, k),
            d,
            tip: charttip::part(
                &format!("{}, {}", ch.langs()[lang], fmt::day(ch.time(pts[last]))),
                rows[last][lang].max(0) as u64,
                totals[last].max(0) as u64,
                charttip::LINES,
            ),
        });
        base = top;
    }
    // Date labels under the plot.
    let dates = (0..=4u8)
        .map(|s| {
            let want = LEFT + pw * f64::from(s) / 4.0;
            let j = xs
                .iter()
                .enumerate()
                .min_by(|a, b| (a.1 - want).abs().total_cmp(&(b.1 - want).abs()))
                .map_or(0, |m| m.0);
            DateLabel {
                x: P(xs[j]),
                anchor: match s {
                    0 => "start",
                    4 => "end",
                    _ => "middle",
                },
                text: fmt::day(ch.time(pts[j])),
            }
        })
        .collect();
    let sel = view.range.map(|(a, b)| {
        let ja = pts.iter().position(|p| *p == a).unwrap_or(0);
        let jb = pts.iter().position(|p| *p == b).unwrap_or(pts.len() - 1);
        (P(xs[ja]), P((xs[jb] - xs[ja]).max(2.0)))
    });
    // One row per band, read where the bands' tips read: the first and the newest shown point.
    let last = pts.len() - 1;
    let row = |label: &str, first: i64, now: i64, total: i64| Row {
        label: label.to_owned(),
        cells: vec![
            fmt::thousands(first.max(0)),
            fmt::thousands(now.max(0)),
            charttip::share(now.max(0) as u64, total.max(0) as u64),
        ],
    };
    let table = Table {
        head: vec![
            "Language".to_owned(),
            fmt::day(ch.time(pts[0])),
            fmt::day(ch.time(pts[last])),
            "Share".to_owned(),
        ],
        rows: (0..k)
            .rev()
            .map(|lang| row(&ch.langs()[lang], rows[0][lang], rows[last][lang], totals[last]))
            .collect(),
        total: Some(row("Total", totals[0], totals[last], totals[last])),
    };
    let summary = if n == 1 {
        format!("1 commit on {}. {lead}", fmt::day(ch.time(0)))
    } else {
        format!(
            "{} from {} to {}. {lead}",
            fmt::count(n, "commit", "commits"),
            fmt::day(ch.time(0)),
            fmt::day(ch.time(n - 1))
        )
    };
    let area = Area {
        fig: charts::fig(AREA_NAME, summary.trim_end().to_owned(), table),
        right: P(W - RIGHT),
        ticks: grid,
        bands,
        dates,
        none: sel.is_none(),
        sel: sel.unwrap_or((P(0.0), P(0.0))),
        ph: P(ph),
        xh_y2: P(TOP_PAD + ph),
    };

    let data = PlotData {
        d: pts.iter().map(|i| fmt::date(ch.time(*i), ch.offset(*i))).collect(),
        i: &pts,
        l: rows,
        n,
        names: ch.langs(),
        s: pts.iter().map(|i| subj.of(*i)).collect(),
        share: view.share,
        view: view.href(|v| {
            v.range = None;
            v.picked = false;
        }),
        x: xs.iter().map(|x| (x * 10.0).round() / 10.0).collect(),
    };
    // Strings and numbers only: serialization cannot fail.
    let json = serde_json::to_string(&data).unwrap_or_default().replace('<', "\\u003c");
    (area, json)
}

fn range_card(ch: &Chain, view: View, subj: &Subjects) -> RangeCard {
    let n = ch.len();
    if view.held == Held::Nothing {
        return RangeCard {
            title: format!("Last {}", view.period.label().to_lowercase()),
            empty: Some(Empty::new("No commits in this period.")),
            ..RangeCard::default()
        };
    }
    let (from, to) = view.range.unwrap_or((0, n - 1));
    let base = ch.before(from);
    let end = &ch.lines(to);
    let count = to - from + 1;
    let (added, removed) = (ch.added, ch.removed);
    let net = added as i64 - removed as i64;
    let k = ch.langs().len();
    let deltas: Vec<i64> = (0..k).map(|i| end[i] - base[i]).collect();
    let big = deltas.iter().map(|d| d.unsigned_abs()).max().unwrap_or(1).max(1);
    let end_total: i64 = end.iter().sum::<i64>().max(1);

    let rows = (0..k)
        .map(|i| {
            let d = deltas[i];
            RangeRow {
                class: cat(i, k),
                name: ch.langs()[i].clone(),
                before: fmt::thousands(base[i].max(0)),
                after: fmt::thousands(end[i].max(0)),
                share: share_label(end[i].max(0) as f64 / end_total as f64),
                up: d >= 0,
                change: signed(d),
                w: (d.unsigned_abs() * 100 / big).max(u64::from(d != 0)),
            }
        })
        .collect();
    let shown = count.min(COMMIT_LIST);
    RangeCard {
        title: if view.picked {
            "Selected range".to_owned()
        } else if view.period.days().is_some() {
            format!("Last {}", view.period.label().to_lowercase())
        } else {
            "Whole history".to_owned()
        },
        note: if view.picked || view.period.days().is_none() {
            ""
        } else {
            "From the first commit dated in the period to the newest."
        },
        empty: None,
        from: fmt::day(ch.time(from)),
        to: fmt::day(ch.time(to)),
        count: fmt::thousands(count),
        count_noun: fmt::noun(count, "commit", "commits"),
        added: fmt::thousands(added),
        removed: fmt::thousands(removed),
        net_up: net >= 0,
        net: signed(net),
        rows,
        list_title: fmt::count(count, "commit", "commits"),
        commits: (to + 1 - shown..=to)
            .rev()
            .map(|i| RangeCommit {
                sha: fmt::sha_link(ch.id(i)),
                subject: subj.of(i),
                day: fmt::day(ch.time(i)),
            })
            .collect(),
        cut: (shown < count).then_some(shown),
    }
}

fn languages_now(ch: &Chain) -> Now {
    let n = ch.len();
    let k = ch.langs().len();
    let last = &ch.lines(n - 1);
    let back = ch.before(n.saturating_sub(10));
    let total: i64 = last.iter().map(|v| (*v).max(0)).sum::<i64>().max(1);
    let bar = (0..k)
        .filter_map(|i| {
            let f = last[i].max(0) as f64 / total as f64;
            (f > 0.0).then(|| SharePart {
                class: cat(i, k),
                w: (f * 100.0).round().clamp(1.0, 100.0) as u32,
                tip: charttip::part(
                    &ch.langs()[i],
                    last[i].max(0) as u64,
                    total.max(0) as u64,
                    charttip::LINES,
                ),
            })
        })
        .collect();
    let shown = |v: i64| u64::try_from(v.max(0)).unwrap_or(0);
    let row = |label: &str, v: i64| Row {
        label: label.to_owned(),
        cells: vec![fmt::thousands(shown(v)), charttip::share(shown(v), shown(total))],
    };
    let table = Table {
        head: ["Language", "Lines", "Share"].map(str::to_owned).to_vec(),
        rows: (0..k).filter(|i| last[*i] > 0).map(|i| row(&ch.langs()[i], last[i])).collect(),
        total: Some(row("Total", total)),
    };
    let summary = (0..k).filter(|i| last[*i] > 0).max_by_key(|i| last[*i]).map_or_else(
        || "No lines.".to_owned(),
        |i| {
            format!(
                "{} at the newest commit, the most in {} with {}.",
                fmt::count(shown(total), "line", "lines"),
                ch.langs()[i],
                charttip::share(shown(last[i]), shown(total))
            )
        },
    );
    let fig: Fig = charts::fig("Share of lines by language", summary, table);
    let window = if n <= 10 {
        format!("All {}", fmt::count(n, "commit", "commits"))
    } else {
        "Last 10 commits".to_owned()
    };
    let step = (n / 40).max(1);
    let rows = (0..k)
        .map(|i| {
            let mut series: Vec<i64> =
                (0..n).step_by(step).map(|c| ch.lines(c)[i].max(0)).collect();
            series.push(last[i].max(0));
            let maxv = series.iter().copied().fold(1i64, i64::max);
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
            NowRow {
                class: cat(i, k),
                name: ch.langs()[i].clone(),
                lines: fmt::thousands(last[i].max(0)),
                share: share_label(last[i].max(0) as f64 / total as f64),
                spark_class: cat_s(i, k),
                points: pts.join(" "),
                up: d >= 0,
                change: if d == 0 { "0".to_owned() } else { signed(d) },
            }
        })
        .collect();
    Now { fig, bar, window, rows }
}

fn seg(label: &str, items: &[(&str, bool, String)]) -> Seg {
    Seg::links(
        label,
        items.iter().map(|(text, on, href)| Choice::new(*text, href.clone(), *on)).collect(),
    )
    .no_caption()
}

pub(super) fn body(ch: &Chain, view: View, subj: &Subjects) -> Body {
    if ch.len() == 0 {
        return Body { empty: Some(Empty::new("No commits match these filters.")), shown: None };
    }
    let period = Seg::links(
        "Period",
        <Period as StringEnum>::ALL
            .iter()
            .map(|p| {
                let href = view.href(|v| {
                    v.period = *p;
                    v.picked = false;
                    v.range = None;
                });
                Choice::new(p.label(), href, !view.picked && *p == view.period)
            })
            .collect(),
    )
    .no_caption();
    let last = ch.lines(ch.len() - 1);
    let total: i64 = last.iter().sum();
    let positive: i64 = last.iter().map(|v| (*v).max(0)).sum::<i64>().max(1);
    let top = last
        .iter()
        .enumerate()
        .filter(|(i, _)| ch.langs()[*i] != "Other")
        .max_by_key(|(_, v)| **v)
        .map(|(i, v)| (ch.langs()[i].clone(), share_label((*v).max(0) as f64 / positive as f64)));
    let lead = top
        .as_ref()
        .map(|(name, share)| crate::lead::languages(name, share, total))
        .unwrap_or_default();
    let counted = last.iter().filter(|v| **v > 0).count();
    let kpis = vec![
        Kpi("Languages", fmt::thousands(counted)),
        Kpi("Lines", fmt::thousands(total.max(0))),
        Kpi("Largest", top.as_ref().map_or_else(|| "-".to_owned(), |t| t.0.clone())),
        Kpi("Its share", top.map_or_else(|| "-".to_owned(), |t| t.1)),
    ];
    let k = ch.langs().len();
    let (area, json) = area_chart(ch, view, subj, &lead);
    Body {
        empty: None,
        shown: Some(Shown {
            lead,
            kpis,
            period,
            axis: seg(
                "X axis",
                &[
                    ("Per commit", !view.time, view.href(|v| v.time = false)),
                    ("Over time", view.time, view.href(|v| v.time = true)),
                ],
            ),
            scale: seg(
                "Scale",
                &[
                    ("Lines", !view.share, view.href(|v| v.share = false)),
                    ("Share", view.share, view.href(|v| v.share = true)),
                ],
            ),
            total: fmt::count(total.max(0), "line", "lines"),
            legend: ch.langs().iter().enumerate().map(|(i, l)| (cat(i, k), l.clone())).collect(),
            area,
            json,
            range: range_card(ch, view, subj),
            now: languages_now(ch),
        }),
    }
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
