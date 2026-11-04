//! Membership, parity and the golden moves of the pulse, rhythm and contributor counts.

use super::Period;
use crate::insights::fixtures::{file, rec};
use crate::{Insights, Options};

/// Commits near UTC midnight and around every window edge, in several zone offsets.
fn edge_records() -> Vec<crate::CommitRecord> {
    let offsets = [-12 * 3600, -5 * 3600, 0, 3600, 5 * 3600 + 1800, 14 * 3600];
    let mut out = Vec::new();
    for day in 0..400i64 {
        for (k, off) in offsets.iter().enumerate() {
            if (day + k as i64) % 3 != 0 {
                continue;
            }
            let mut r = rec(&format!("c{day}-{k}"), "Ann", day, vec![]);
            // 23:30, 00:30 or noon UTC.
            let second = [23 * 3600 + 1800, 1800, 12 * 3600][(day as usize + k) % 3];
            r.time = r.time - 12 * 3600 + second;
            r.offset = *off;
            out.push(r);
        }
    }
    out
}

/// Series, weekday, hour, punch card and pulse table count one set of commits.
#[test]
fn every_section_counts_one_set_of_commits() {
    let recs = edge_records();
    let ins = Insights::new(&recs, Options::default(), None);
    let day0 = crate::insights::days_from_civil(2024, 1, 1);
    let ranges = [
        None,
        Some((day0 + 100, day0 + 106)),
        Some((day0 + 200, day0 + 200)),
        Some((day0 + 390, day0 + 420)),
    ];
    for period in [
        Period::Week,
        Period::Month,
        Period::Quarter,
        Period::Year,
        Period::All,
    ] {
        for days in ranges {
            let r = ins.rhythm_in(period, days);
            let series: u32 = r.series.iter().map(|p| p.commits).sum();
            let weekday: u32 = r.by_weekday.iter().sum();
            let hour: u32 = r.by_hour.iter().sum();
            let punch: u32 = r.punch_card.iter().flatten().sum();
            let kpi = ins.period_summary_in(period, days).current.commits as u32;
            let at = format!("{period:?} {days:?}");
            assert_eq!(series, weekday, "series vs weekday, {at}");
            assert_eq!(series, hour, "series vs hour, {at}");
            assert_eq!(series, punch, "series vs punch, {at}");
            assert_eq!(series, kpi, "series vs table, {at}");
            assert_eq!(
                ins.has_commits(period),
                days.is_some() || series > 0,
                "{at}"
            );
        }
    }
}

#[test]
fn pulse_compares_with_previous_window() {
    // Day 0 = Monday 2024-01-01. Newest commit on day 20.
    let recs = vec![
        rec("a", "Ann", 0, vec![file("x.rs", 5, 1, 4)]),
        rec(
            "b",
            "Bob",
            10,
            vec![file("x.rs", 3, 0, 7), file("Cargo.lock", 99, 0, 99)],
        ),
        rec("c", "Ann", 20, vec![file("y.rs", 2, 2, 2)]),
    ];
    let ins = Insights::new(&recs, Options::default(), None);
    let s = ins.period_summary(Period::Month);
    assert_eq!(
        (s.current.commits, s.current.authors, s.current.files),
        (3, 2, 2)
    );
    // Lock files are generated and left out.
    assert_eq!((s.current.additions, s.current.deletions), (10, 3));
    assert_eq!(s.previous.as_ref().map(|p| p.commits), Some(0));
    let w = ins.period_summary(Period::Week);
    assert_eq!(w.current.commits, 1);
    // The 7 days before the newest week include the commit on day 10.
    assert_eq!(w.previous.map(|p| p.commits), Some(1));
    assert!(ins.period_summary(Period::All).previous.is_none());
}

#[test]
fn rhythm_is_dense_and_local() {
    let mut recs = vec![rec("a", "Ann", 0, vec![]), rec("b", "Ann", 15, vec![])];
    recs[1].offset = 3600 * 13; // 12:00 UTC is 01:00 the next day locally
    let r = Insights::new(&recs, Options::default(), None).rhythm(Period::All);
    // 16 local days from Mon 2024-01-01 to Tue 2024-01-16, per day.
    assert_eq!(r.series.len(), 17);
    assert_eq!(r.series[1].commits, 0);
    assert_eq!(r.series[16].commits, 1);
    assert_eq!(r.punch_card[0][12], 1); // Monday noon
    assert_eq!(r.punch_card[2][1], 1); // day 15 is a Tuesday, 12:00 + 13h is Wednesday 01:00
    assert_eq!(r.punch_max, 1);
}

#[test]
fn rhythm_in_limits_weekdays_and_keeps_the_series_and_bucket_ends() {
    let recs = vec![rec("a", "Ann", 0, vec![]), rec("b", "Ann", 3, vec![])];
    let ins = Insights::new(&recs, Options::default(), None);
    let all = ins.rhythm(Period::All);
    let day0 = crate::insights::days_from_civil(2024, 1, 1);
    let one = ins.rhythm_in(Period::All, Some((day0, day0)));
    assert_eq!(one.by_weekday.iter().sum::<u32>(), 1);
    assert_eq!(one.series.iter().map(|p| p.commits).sum::<u32>(), 1);
    assert_eq!(one.series.len(), 1);
    assert_eq!(all.ends.len(), all.series.len());
    assert_eq!(all.ends[0], all.series[0].key);
}

#[test]
fn contributors_have_series_and_flags() {
    let recs = vec![
        rec("a", "Ann", 0, vec![file("x.rs", 5, 1, 4)]),
        rec("b", "dependabot[bot]", 8, vec![]),
        rec("c", "Ann", 16, vec![file("x.rs", 1, 0, 5)]),
        rec("d", "Cy", 16, vec![]),
    ];
    let ins = Insights::new(
        &recs,
        Options {
            include_bots: true,
            ..Options::default()
        },
        None,
    );
    let rep = ins.contributors(Period::All);
    assert_eq!(rep.labels.len(), rep.contributors[0].series.len());
    assert_eq!(rep.contributors[0].name, "Ann");
    assert_eq!(rep.contributors[0].series.iter().sum::<u32>(), 2);
    assert_eq!(rep.contributors[0].series[0], 1);
    assert_eq!(rep.contributors[0].additions, 6);
    assert!(rep.contributors.iter().any(|c| c.bot));
    let m = ins.contributors(Period::Week);
    assert_eq!(m.labels.len(), 7);
    assert!(
        m.contributors
            .iter()
            .all(|c| c.first_time || c.name == "Ann")
    );
    assert!(
        m.contributors
            .iter()
            .any(|c| c.name == "Cy" && c.first_time)
    );
}

/// Newest commit Sunday day 20 at noon UTC, so 1w counts local days 14 to 20. `early` sits
/// on local day 13 although its UTC time is inside the old instant window, `late` on
/// local day 14 although its UTC time is before that window.
fn week_edges() -> Vec<crate::CommitRecord> {
    let mut early = rec("early", "Eve", 13, vec![file("e.rs", 3, 0, 3)]);
    early.time += 8 * 3600; // 20:00 UTC on day 13, 10:00 local at -1000
    early.offset = -10 * 3600;
    let mut late = rec("late", "Lu", 13, vec![file("l.rs", 2, 0, 2)]);
    late.time -= 3600; // 11:00 UTC on day 13, 01:00 local on day 14 at +1400
    late.offset = 14 * 3600;
    let mut v = vec![
        rec("a", "Ann", 0, vec![file("x.rs", 1, 0, 1)]),
        late,
        early,
        rec(
            "bot",
            "dependabot[bot]",
            18,
            vec![file("Cargo.toml", 1, 0, 1)],
        ),
        rec("z", "Ann", 20, vec![file("x.rs", 1, 0, 2)]),
    ];
    v.sort_by_key(|r| r.time);
    v
}

/// The per-day bars hold exactly the 7 local days, the day before is not in bar 0.
#[test]
fn the_bars_leave_out_the_day_before_the_first() {
    let recs = week_edges();
    let ins = Insights::new(&recs, Options::default(), None);
    let r = ins.rhythm(Period::Week);
    assert_eq!(r.series.len(), 7);
    let per: Vec<u32> = r.series.iter().map(|p| p.commits).collect();
    assert_eq!(per, [1, 0, 0, 0, 0, 0, 1], "late on bar 0, early nowhere");
}

/// The KPI, the punch card and the weekday split count the same local days.
#[test]
fn kpis_and_punch_count_the_local_days() {
    let recs = week_edges();
    let ins = Insights::new(&recs, Options::default(), None);
    let s = ins.period_summary(Period::Week);
    assert_eq!((s.current.commits, s.current.authors), (2, 2));
    assert_eq!((s.current.files, s.current.additions), (2, 3));
    let r = ins.rhythm(Period::Week);
    assert_eq!(r.punch_card.iter().flatten().sum::<u32>(), 2);
    assert_eq!(r.by_weekday.iter().sum::<u32>(), 2);
    // The before column is the 7 local days before: early (day 13) is there.
    assert_eq!(s.previous.map(|p| p.commits), Some(1));
}

/// bots=off leaves bots out of the contributor rows, as every other count does.
#[test]
fn contributors_follow_the_bot_option() {
    let recs = week_edges();
    let off = Insights::new(&recs, Options::default(), None);
    let names = |i: &Insights<'_>| -> Vec<String> {
        i.contributors(Period::All)
            .contributors
            .into_iter()
            .map(|c| c.name)
            .collect()
    };
    assert!(
        !names(&off).iter().any(|n| n.contains("[bot]")),
        "{:?}",
        names(&off)
    );
    let on = Options {
        include_bots: true,
        ..Options::default()
    };
    let on = Insights::new(&recs, on, None);
    assert!(names(&on).iter().any(|n| n.contains("[bot]")));
}

/// A person's commits are the local days of the period, and the sparkline adds up to them.
#[test]
fn sparklines_count_the_local_days() {
    let recs = week_edges();
    let ins = Insights::new(&recs, Options::default(), None);
    let rep = ins.contributors(Period::Week);
    let got: Vec<(String, usize, u32)> = rep
        .contributors
        .iter()
        .map(|c| (c.name.clone(), c.commits, c.series.iter().sum()))
        .collect();
    assert_eq!(
        got,
        [("Ann".to_owned(), 1, 1), ("Lu".to_owned(), 1, 1)],
        "early is out, late is in"
    );
    assert_eq!(rep.contributors[1].series[0], 1);
}

/// A 1w summary with "now" at 12:00 UTC spans exactly the 7 counted local days, not the
/// instant window that touched 8 dates.
#[test]
fn the_summary_span_is_the_counted_days() {
    let recs = week_edges();
    let ins = Insights::new(&recs, Options::default(), None);
    let s = ins.period_summary(Period::Week);
    let day = crate::insights::DAY;
    let from = s.from.expect("a fixed period has a start");
    assert_eq!(from.rem_euclid(day), 0, "starts at a midnight");
    assert_eq!(s.to.rem_euclid(day), 0, "ends at a midnight");
    assert_eq!((s.to - from) / day, 7, "7 dates, from {from} to {}", s.to);
    let day0 = crate::insights::days_from_civil(2024, 1, 1);
    assert_eq!(from / day, day0 + 14);
}

/// `All` counts every commit the scope admits over the whole history, dated after the
/// clock or not, and the clock reading does not change the result.
#[test]
fn all_counts_every_admitted_commit_whatever_the_clock() {
    use crate::feed::{CommitRange, FixedClock, MemorySource};
    use crate::insights::{CountContext, Scope, ScopeParams};
    let mut far = rec("far", "Fay", 0, vec![file("f.rs", 1, 0, 1)]);
    far.time = 4_102_444_800; // 2100-01-01, after any clock this test runs under
    let recs = vec![
        rec("a", "Ann", 0, vec![file("x.rs", 1, 0, 1)]),
        rec("b", "Bo", 10, vec![file("y.rs", 1, 0, 1)]),
        far,
    ];
    let ins = Insights::new(&recs, Options::default(), None);
    assert_eq!(
        ins.period_summary(Period::All).current.commits,
        3,
        "the page"
    );
    let s = Scope::from_params(&ScopeParams::default(), Options::default(), Period::All)
        .expect("scope");
    let src = MemorySource::new(&recs);
    let got: Vec<usize> = [20, 1_000_000]
        .into_iter()
        .map(|d| {
            let cx = CountContext::new(&src, &FixedClock(d * crate::insights::DAY));
            let admitted = ins
                .source()
                .commits_in(CommitRange::All)
                .filter(|r| s.admits(&ins, &cx, r))
                .count();
            let n = ins.counted(&s, cx).totals().commits;
            assert_eq!(n, admitted, "totals == admitted at clock day {d}");
            n
        })
        .collect();
    assert_eq!(got, [3, 3], "independent of the clock");
}
