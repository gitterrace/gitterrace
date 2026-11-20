//! The text form of each chart: the summary sentence it is described by and its values as a
//! table. The table carries what the tips carry: every mark's value is a cell, and a total a
//! tip names is a cell of the total row.

use crate::charttip::{self, Unit};
use crate::fmt;

use crate::charts::DAYS;
use crate::charts::view::{Fig, Row, Table};

/// The id stem shared by a chart's svg, summary and table, from its name.
pub(crate) fn chart_id(name: &str) -> String {
    let mut id = String::from("chart");
    for w in name.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        id.push('-');
        id.push_str(&w.to_ascii_lowercase());
    }
    id
}

pub(crate) fn fig(name: &str, summary: String, table: Table) -> Fig {
    Fig { id: chart_id(name), name: name.to_owned(), summary, table }
}

fn span(first: &str, last: &str) -> String {
    if first == last { format!("for {first}") } else { format!("from {first} to {last}") }
}

/// `word` with its first letter in upper case, for a column head.
fn cap(word: &str) -> String {
    let mut c = word.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect::<String>())
}

fn heads(cols: &[&str]) -> Vec<String> {
    cols.iter().map(|c| (*c).to_owned()).collect()
}

fn sum_row(label: &str, vals: &[u64]) -> Row {
    let mut cells: Vec<String> = vals.iter().map(|v| fmt::thousands(*v)).collect();
    cells.push(fmt::thousands(vals.iter().sum::<u64>()));
    Row { label: label.to_owned(), cells }
}

/// A single-series chart: its span, its total and its highest column, and one row per column
/// with the total its tips share.
pub(crate) fn bars(name: &str, points: &[(String, u64)], unit: Unit) -> Fig {
    let all: u64 = points.iter().map(|p| p.1).sum();
    let summary = match (points.first(), points.last()) {
        (Some(first), Some(last)) => {
            let total = fmt::count(all, unit.0, unit.1);
            let span = span(&first.0, &last.0);
            // The first of equal maxima, as a reader scanning left to right finds it.
            let top = points.iter().fold(first, |best, p| if p.1 > best.1 { p } else { best });
            if all == 0 {
                format!("{total} {span}.")
            } else {
                format!(
                    "{total} {span}, the most in {} with {}.",
                    top.0,
                    fmt::count(top.1, unit.0, unit.1)
                )
            }
        }
        _ => "No data.".to_owned(),
    };
    let table = Table {
        head: vec!["Column".to_owned(), cap(unit.1), "Share".to_owned()],
        rows: points
            .iter()
            .map(|(label, v)| Row {
                label: label.clone(),
                cells: vec![fmt::thousands(*v), charttip::share(*v, all)],
            })
            .collect(),
        total: Some(Row {
            label: "Total".to_owned(),
            cells: vec![fmt::thousands(all), charttip::share(all, all)],
        }),
    };
    fig(name, summary, table)
}

/// Stacked columns: `series` names the parts, `cols` holds each column's part values.
pub(crate) fn stacked(name: &str, labels: &[&str], series: &[&str], cols: &[Vec<u64>]) -> Fig {
    let sums: Vec<u64> = (0..series.len()).map(|k| cols.iter().map(|c| c[k]).sum()).collect();
    let all: u64 = sums.iter().sum();
    let mut head = vec!["Column".to_owned()];
    head.extend(series.iter().map(|s| (*s).to_owned()));
    head.push("Commits".to_owned());
    let table = Table {
        head,
        rows: labels.iter().zip(cols).map(|(l, c)| sum_row(l, &c[..series.len()])).collect(),
        total: Some(sum_row("Total", &sums)),
    };
    let summary = match (labels.first(), labels.last(), series.first()) {
        (Some(a), Some(b), Some(top)) if all > 0 => format!(
            "{} {}, most of them {top} with {} ({}).",
            fmt::count(all, "commit", "commits"),
            span(a, b),
            fmt::thousands(sums[0]),
            charttip::share(sums[0], all)
        ),
        _ => "No commits.".to_owned(),
    };
    fig(name, summary, table)
}

pub(crate) fn diverging(name: &str, points: &[(String, u64, u64)]) -> Fig {
    let (added, removed): (u64, u64) = points.iter().fold((0, 0), |(a, d), p| (a + p.1, d + p.2));
    let summary = match (points.first(), points.last()) {
        (Some(a), Some(b)) => format!(
            "{} added and {} removed {}.",
            fmt::count(added, "line", "lines"),
            fmt::thousands(removed),
            span(&a.0, &b.0)
        ),
        _ => "No changes.".to_owned(),
    };
    let row = |label: &str, a: u64, d: u64| Row {
        label: label.to_owned(),
        cells: vec![fmt::thousands(a), fmt::thousands(d), fmt::thousands(a + d)],
    };
    let table = Table {
        head: heads(&["Column", "Added", "Removed", "Lines"]),
        rows: points.iter().map(|(l, a, d)| row(l, *a, *d)).collect(),
        total: Some(row("Total", added, removed)),
    };
    fig(name, summary, table)
}

pub(crate) fn percent_line(name: &str, points: &[(String, Option<f64>)]) -> Fig {
    let read: Vec<(&str, f64)> =
        points.iter().filter_map(|(l, v)| v.map(|v| (l.as_str(), v))).collect();
    let summary = match (read.first(), read.last()) {
        (Some(a), Some(b)) => {
            let hi = read.iter().fold(a, |best, p| if p.1 > best.1 { p } else { best });
            format!(
                "Rework went from {} in {} to {} in {}, highest at {} in {}.",
                charttip::percent(a.1),
                a.0,
                charttip::percent(b.1),
                b.0,
                charttip::percent(hi.1),
                hi.0
            )
        }
        _ => "No changes to read a share from.".to_owned(),
    };
    let table = Table {
        head: heads(&["Column", "Rework"]),
        rows: points
            .iter()
            .map(|(l, v)| Row {
                label: l.clone(),
                cells: vec![v.map_or_else(|| "-".to_owned(), charttip::percent)],
            })
            .collect(),
        total: None,
    };
    fig(name, summary, table)
}

/// The scatter plot: one row per file, `(name, changes, lines, authors)`.
pub(crate) fn scatter(name: &str, x_title: &str, dots: &[(&str, u64, u64, u64)]) -> Fig {
    let summary = dots.iter().max_by_key(|d| d.1).map_or_else(
        || "No files.".to_owned(),
        |top| {
            format!(
                "{} plotted by {x_title} against lines, {} the most often changed with {}.",
                fmt::count(dots.len(), "file", "files"),
                top.0,
                fmt::count(top.1, "change", "changes")
            )
        },
    );

    let table = Table {
        head: heads(&["File", "Changes", "Lines", "Authors"]),
        rows: dots
            .iter()
            .map(|d| Row {
                label: d.0.to_owned(),
                cells: vec![fmt::thousands(d.1), fmt::thousands(d.2), fmt::thousands(d.3)],
            })
            .collect(),
        total: None,
    };
    fig(name, summary, table)
}

/// The weekday by hour grid, with its totals by day and by hour.
pub(crate) fn heatmap(name: &str, cells: &[[u32; 24]; 7]) -> Fig {
    let all: u64 = cells.iter().flatten().map(|v| u64::from(*v)).sum();
    let mut busiest = (0, 0, 0u32);
    for (d, row) in cells.iter().enumerate() {
        for (hr, v) in row.iter().enumerate() {
            if *v > busiest.2 {
                busiest = (d, hr, *v);
            }
        }
    }
    let summary = if all == 0 {
        "No commits.".to_owned()
    } else {
        format!(
            "{} by weekday and hour, the most on {} at {:02}:00 with {}.",
            fmt::count(all, "commit", "commits"),
            DAYS[busiest.0],
            busiest.1,
            fmt::count(busiest.2, "commit", "commits")
        )
    };
    let mut head = vec!["Day".to_owned()];
    head.extend((0..24).map(|h| format!("{h:02}")));
    head.push("Commits".to_owned());
    let by_hour: Vec<u64> = (0..24).map(|h| cells.iter().map(|r| u64::from(r[h])).sum()).collect();
    let table = Table {
        head,
        rows: cells.iter().zip(DAYS).map(|(r, d)| sum_row(d, &r.map(u64::from))).collect(),
        total: Some(sum_row("Total", &by_hour)),
    };
    fig(name, summary, table)
}
