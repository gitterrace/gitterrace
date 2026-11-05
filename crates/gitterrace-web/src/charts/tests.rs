use crate::charts::{
    Dot, bars, diverging, heatmap, label_every, linked_bars, pct, percent_line, scatter, ticks,
};
use crate::charttip::COMMITS;

#[test]
fn bars_escape_labels_and_scale() {
    let svg = bars(&[("<a>".to_owned(), 5), ("b".to_owned(), 0)], COMMITS);
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

/// A hover band is opaque on hover; drawn after a mark it hides the mark. In every
/// chart, the last hover band must come before the first data mark it could cover.
#[test]
fn hover_bands_paint_under_the_data() {
    let line = percent_line(&[
        ("a".to_owned(), Some(0.2)),
        ("b".to_owned(), Some(0.5)),
        ("c".to_owned(), None),
    ]);
    let last_hit = line.rfind(r#"class="hit""#).expect("hit");
    let path = line.find(r#"class="line""#).expect("line");
    assert!(last_hit < path, "hover band drawn over the line");
    for svg in [
        bars(&[("a".to_owned(), 3), ("b".to_owned(), 1)], COMMITS),
        diverging(&[("a".to_owned(), 3, 1)], "t"),
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

/// The code frequency chart once labelled every 10,000 on both sides, "-30,000" included.
#[test]
fn diverging_axis_has_few_labels() {
    let svg = diverging(&[("a".to_owned(), 184_815, 95_395)], "t");
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
    let svg = bars(&pts, COMMITS);
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
    for svg in [linked_bars(&pts, &links), scatter(&dots, "changes")] {
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
        !bars(&pts, COMMITS).contains("tabindex"),
        "a chart without links takes no stop"
    );
}
