use crate::charts::{
    Dot, bars, diverging, heatmap, label_every, linked_bars, pct, percent_line, scatter,
    split_ticks, ticks,
};
use crate::charttip::COMMITS;

#[test]
fn bars_escape_labels_and_scale() {
    let svg = bars(&[("<a>\"&".to_owned(), 5), ("b".to_owned(), 0)], COMMITS).to_string();
    assert!(svg.contains("&lt;a&gt;&quot;&amp;"), "{svg}");
    assert!(svg.contains("&lt;a&gt;"));
    assert!(!svg.contains("<a>"));
    assert_eq!(svg.matches(r#"class="bar""#).count(), 2);
}

#[test]
fn heatmap_has_all_cells() {
    let mut cells = [[0; 24]; 7];
    cells[0][0] = 3;
    let svg = heatmap(&cells, 3).to_string();
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
    ])
    .to_string();
    let last_hit = line.rfind(r#"class="hit""#).expect("hit");
    let path = line.find(r#"class="line""#).expect("line");
    assert!(last_hit < path, "hover band drawn over the line");
    for svg in [
        bars(&[("a".to_owned(), 3), ("b".to_owned(), 1)], COMMITS).to_string(),
        diverging(&[("a".to_owned(), 3, 1)], "t").to_string(),
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

/// 1, 2 or 5 times a power of ten.
fn nice(step: u64) -> bool {
    let mut s = step;
    while s >= 10 && s.is_multiple_of(10) {
        s /= 10;
    }
    matches!(s, 1 | 2 | 5)
}

/// For every max and tick budget: the step is nice, the top covers the range with less than
/// one step to spare, and the ticks never number more than the budget.
#[test]
fn ticks_are_nice_cover_the_range_and_stay_few() {
    let maxes = (0..2000u64).chain([
        9_999,
        10_000,
        10_001,
        30_000,
        95_395,
        184_815,
        999_999,
        1 << 40,
    ]);
    for max in maxes {
        for n in 2..=6 {
            let (step, top) = ticks(max, n);
            assert!(nice(step), "max {max}: step {step}");
            assert!(
                top >= max.max(1) && top - max.max(1) < step,
                "max {max}: top {top}"
            );
            assert_eq!(top % step, 0, "max {max}");
            assert!(
                top / step < n.max(2),
                "max {max}, n {n}: {} ticks",
                top / step + 1
            );
        }
    }
}

/// For every pair of side maxima: one nice step for both sides, each side covered with less
/// than a step to spare, and at most 5 ticks in all.
#[test]
fn split_ticks_cover_both_sides_on_one_step() {
    let maxes: Vec<u64> = (0..300u64)
        .chain([999, 1_000, 1_001, 9_999, 30_000, 95_395, 184_815, 1 << 40])
        .collect();
    for &up in &maxes {
        for &down in maxes.iter().step_by(7) {
            let (step, a, d) = split_ticks(up, down, 5);
            assert!(nice(step), "{up}/{down}: step {step}");
            for (top, max) in [(a, up), (d, down)] {
                assert_eq!(top % step, 0, "{up}/{down}");
                assert!(
                    top >= max.max(1) && top - max.max(1) < step,
                    "{up}/{down}: top {top} for {max}"
                );
            }
            // Ticks above zero, below it, and zero itself: at most 5.
            assert!(a / step + d / step < 5, "{up}/{down}: too many ticks");
        }
    }
}

/// The zero line follows the data: with far more added than removed, the added side is taller.
#[test]
fn diverging_zero_line_follows_the_data() {
    let svg = diverging(&[("a".to_owned(), 4_000, 900)], "t").to_string();
    // One step of 2,000: added reaches 4,000, removed stops at its own first step.
    assert!(svg.contains(">4,000<"), "{svg}");
    assert!(svg.contains(">\u{2212}2,000<"), "{svg}");
    assert!(!svg.contains(">\u{2212}4,000<"), "{svg}");
}

/// The code frequency chart once labelled every 10,000 on both sides, "-30,000" included.
#[test]
fn diverging_axis_has_few_labels() {
    let svg = diverging(&[("a".to_owned(), 184_815, 95_395)], "t").to_string();
    let labels = svg.matches(r#"text-anchor="end""#).count();
    assert!(labels <= 5, "{labels} labels");
    assert!(!svg.contains("-30,000"));
}

#[test]
fn labels_step_by_calendar() {
    assert_eq!(label_every(6, true), 8);
    assert_eq!(label_every(3, true), 4);
    assert_eq!(label_every(1, true), 1);
    assert_eq!(label_every(3, false), 5);
    assert_eq!(label_every(12, false), 20);
    let keys: Vec<String> = (0..40)
        .map(|i| format!("Q{} {}", i % 4 + 1, 2016 + i / 4))
        .collect();
    let pts: Vec<(String, u64)> = keys.iter().map(|k| (k.clone(), 1)).collect();
    let svg = bars(&pts, COMMITS).to_string();
    let shown: Vec<&str> = svg
        .split(r#"text-anchor="middle">"#)
        .skip(1)
        .map(|t| t.split('<').next().unwrap_or_default())
        .collect();
    assert!(
        !shown.is_empty() && shown.iter().all(|t| t.starts_with("Q1")),
        "{shown:?}"
    );
}

#[test]
fn a_repeated_day_is_labelled_once() {
    let pts: Vec<(String, u64)> = (0..12)
        .map(|i| (format!("2026-11-{:02}", 1 + i / 4), 1))
        .collect();
    let svg = bars(&pts, COMMITS).to_string();
    let shown: Vec<&str> = svg
        .split(r#"class="ax""#)
        .skip(1)
        .map(|t| {
            t.split('>')
                .nth(1)
                .and_then(|t| t.split('<').next())
                .unwrap_or_default()
        })
        .collect();
    let mut uniq = shown.clone();
    uniq.dedup();
    assert!(!shown.is_empty() && uniq == shown, "{shown:?}");
}

/// Every chart whose marks are links is one tab stop: the chart takes focus and each link is
/// out of the tab order, reached with the arrow keys instead.
#[test]
fn a_chart_of_links_is_one_tab_stop() {
    let pts: Vec<_> = (0..30).map(|i| (format!("w{i}"), i)).collect();
    let links: Vec<_> = (0..30).map(|i| format!("/r?w={i}")).collect();
    let dots: Vec<_> = (0..30)
        .map(|i| Dot {
            x: i,
            y: i,
            size: 1,
            href: format!("/h/{i}"),
            title: format!("f{i}"),
        })
        .collect();
    for svg in [
        linked_bars(&pts, &links).to_string(),
        scatter(&dots, "changes").to_string(),
    ] {
        let open = svg.split('>').next().expect("svg tag");
        assert!(
            open.contains("tabindex=\"0\"") && open.contains("data-marks"),
            "{open}"
        );
        let anchors = svg.matches("<a ").count();
        assert_eq!(anchors, 30, "{svg}");
        assert_eq!(svg.matches("<a tabindex=\"-1\" ").count(), anchors, "{svg}");
    }
    assert!(
        !bars(&pts, COMMITS).to_string().contains("tabindex"),
        "a chart without links takes no stop"
    );
}

#[test]
fn a_short_per_commit_axis_still_names_its_first_and_last_day() {
    for n in 2..=9 {
        let pts: Vec<(String, u64)> = (0..n)
            .map(|i| (format!("2026-11-{:02}", 1 + i / 3), 1))
            .collect();
        let svg = bars(&pts, COMMITS).to_string();
        let shown: Vec<&str> = svg
            .split(r#"class="ax""#)
            .skip(1)
            .filter_map(|t| t.split('>').nth(1).and_then(|t| t.split('<').next()))
            .filter(|t| t.starts_with("2026") || t.contains("Nov"))
            .collect();
        let first = &pts[0].0;
        let last = &pts[pts.len() - 1].0;
        let named = |d: &str| {
            shown
                .iter()
                .any(|s| d.ends_with(&s[s.len().saturating_sub(2)..]))
        };
        assert!(!shown.is_empty(), "{n}: no day labels in {svg}");
        assert!(named(first), "{n}: first day {first} unnamed in {shown:?}");
        assert!(named(last), "{n}: last day {last} unnamed in {shown:?}");
    }
}
