//! Every chart has a text form: an accessible name, a one-sentence summary it is described by,
//! and its values as a data table with headers. The table carries what the tips carry: each
//! mark's value is a cell, and each total a tip names is a cell of the table's total row.

use gitterrace_testkit::{Fixture, page};

/// The text between `start` and the next `end` after it.
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let at = s.find(start)? + start.len();
    s[at..].find(end).map(|e| &s[at..at + e])
}

/// Text with the five escapes the templates write turned back.
fn unesc(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The text of every `<tag ...>...</tag>` in `s`, tags inside stripped.
fn cells(s: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    s.split(&open)
        .skip(1)
        .filter_map(|c| {
            let body = &c[c.find('>')? + 1..];
            Some(unesc(&body[..body.find(&close)?]))
        })
        .collect()
}

/// The leading value of a tip ("Mon: 12 commits (40%) of 30" gives "12", "May: 42% rework"
/// gives "42%") and the total it names after " of ", if any.
fn tip_numbers(tip: &str) -> (String, Option<String>) {
    // The value is the first part after a ": " that starts with a number: a breakdown tip
    // "Q1: 40 commits: feat 30 (75%)" leads with its column total.
    let after = tip
        .split(": ")
        .skip(1)
        .find(|p| p.starts_with(|c: char| c.is_ascii_digit() || c == '<'))
        .unwrap_or_default();
    let lead: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '%' || *c == '<')
        .collect();
    let total = tip
        .split(" of ")
        .nth(1)
        .map(|t| t.chars().take_while(|c| c.is_ascii_digit() || *c == ',').collect::<String>());
    (lead, total.filter(|t| !t.is_empty()))
}

/// The charts in `s`: svg charts, share bars and shaded grids. A shaded grid is any matrix
/// card, so one written without the chart figure is counted and caught.
fn marks_in(s: &str) -> usize {
    s.matches("<svg class=\"chart").count()
        + s.matches("class=\"sharebar\"").count()
        + s.matches("class=\"matrix").count()
        + s.matches("class=\"card matrix").count()
}

/// What is wrong with the charts of `body`, one line per fault.
fn faults(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let charts = marks_in(body);
    let figures: Vec<&str> = body
        .split("<figure class=\"chart-fig\">")
        .skip(1)
        .map(|f| f.split("</figure>").next().unwrap_or_default())
        .collect();
    let in_figures: usize = figures.iter().map(|f| marks_in(f)).sum();
    if in_figures != charts {
        out.push(format!("{} of {charts} charts have no figure with a table", charts - in_figures));
    }
    for f in figures {
        let open = between(f, "<svg class=\"chart", ">")
            .or_else(|| between(f, "<div class=\"sharebar\"", ">"))
            .or_else(|| between(f, "<div class=\"matrix matrix-chart\"", ">"))
            .unwrap_or_default();
        let name = between(open, "aria-label=\"", "\"").unwrap_or_default();
        if name.is_empty() {
            out.push("a chart without a name".to_owned());
            continue;
        }
        let Some(desc) = between(open, "aria-describedby=\"", "\"") else {
            out.push(format!("{name}: not described by a summary"));
            continue;
        };
        let summary = between(f, &format!("id=\"{desc}\">"), "</figcaption>").unwrap_or_default();
        if summary.trim().is_empty() {
            out.push(format!("{name}: empty or missing summary {desc}"));
        }
        let data = f.rsplit_once("<details").map_or("", |d| d.1);
        let Some(table) = between(data, "<table", "</table>") else {
            out.push(format!("{name}: no data table"));
            continue;
        };
        let head = between(table, "<thead>", "</thead>").unwrap_or_default();
        let rows = between(table, "<tbody>", "</tbody>").unwrap_or_default();
        if !head.contains("<th scope=\"col\"") || !rows.contains("<th scope=\"row\"") {
            out.push(format!("{name}: table without column and row headers"));
        }
        let values = cells(rows, "td");
        let totals = between(table, "<tfoot>", "</tfoot>").map(|t| cells(t, "td"));
        let marks = f.split("<details").next().unwrap_or_default();
        for t in marks.split("data-tip=\"").skip(1) {
            let tip = unesc(t.split('"').next().unwrap_or_default());
            let (lead, total) = tip_numbers(&tip);
            if lead.is_empty() || !values.contains(&lead) {
                out.push(format!("{name}: tip value {lead:?} is no table cell: {tip}"));
            }
            if let (Some(total), Some(row)) = (total, &totals)
                && !row.contains(&total)
            {
                out.push(format!("{name}: tip total {total} is not the table total {row:?}"));
            }
        }
    }
    out
}

#[test]
fn a_planted_chart_without_a_table_is_caught() {
    let bare = concat!(
        r#"<svg class="chart" role="img" aria-label="Activity">"#,
        r#"<g data-tip="Mon: 3 commits (100%) of 3"></g></svg>"#,
    );
    assert_ne!(faults(bare), [] as [std::string::String; 0]);
    let good = concat!(
        r#"<figure class="chart-fig"><svg class="chart" role="img" aria-label="A" "#,
        r#"aria-describedby="chart-a-sum"><g data-tip="Mon: 3 commits (100%) of 3"></g></svg>"#,
        r#"<figcaption class="vh" id="chart-a-sum">3 commits.</figcaption><details><table>"#,
        r#"<thead><tr><th scope="col">Column</th></tr></thead><tbody><tr><th scope="row">"#,
        r#"Mon</th><td class="r">3</td></tr></tbody><tfoot><tr><th scope="row">Total</th>"#,
        r#"<td class="r">3</td></tr></tfoot></table></figure>"#
    );
    assert_eq!(faults(good), Vec::<String>::new());
    // A table whose total drifts from the tips is caught too.
    let drift = good
        .replace(r#"<td class="r">3</td></tr></tfoot>"#, r#"<td class="r">4</td></tr></tfoot>"#);
    assert_ne!(faults(&drift), [] as [std::string::String; 0]);
    let summary_less = good.replace("3 commits.</figcaption>", "</figcaption>");
    assert_ne!(faults(&summary_less), [] as [std::string::String; 0]);
    // A shaded grid outside a figure counts as a chart without a table.
    let grid = concat!(
        r#"<div class="card matrix"><table><tr>"#,
        r#"<td data-tip="a / b: 3 commits (100%) of 3">3</td></tr></table></div>"#,
    );
    assert_ne!(faults(grid), [] as [std::string::String; 0]);
}

#[test]
fn tip_numbers_read_value_and_total() {
    assert_eq!(
        tip_numbers("Mon: 1,203 commits (40%) of 3,000"),
        ("1,203".to_owned(), Some("3,000".to_owned()))
    );
    assert_eq!(
        tip_numbers("Q1 2026: 40 commits: feat 30 (75%), fix 10 (25%)"),
        ("40".to_owned(), None)
    );
    assert_eq!(tip_numbers("May: 42% of changed lines replaced existing code").0, "42%");
}

#[tokio::test]
async fn every_chart_on_the_insights_pages_has_a_summary_and_a_table() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f = Fixture::init(dir.path())
        .write("src/a.rs", "fn main() {}\n")
        .write("x.py", "a = 1\n")
        .commit("feat(core): first");
    for n in 0..4 {
        f = f
            .write("src/a.rs", &format!("fn main() {{ {n} }}\n"))
            .write("x.py", &format!("a = {n}\nb = 2\n"))
            .commit("fix(core): next");
    }
    let router = gitterrace_web::app_router(f.path()).expect("router");
    gitterrace_testkit::oracle::ready(&router).await;
    let mut bad = Vec::new();
    let mut charts = 0;
    for p in ["/stats", "/stats/hotspots", "/stats/languages"] {
        let (status, body) = page(&router, p).await;
        assert_eq!(status, 200, "{p}");
        charts += body.matches("<figure class=\"chart-fig\">").count();
        if p == "/stats" {
            assert!(body.contains("aria-label=\"Type by scope\""), "no type by scope grid");
        }
        bad.extend(faults(&body).into_iter().map(|e| format!("{p}: {e}")));
    }
    assert!(charts >= 6, "only {charts} charts");
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
