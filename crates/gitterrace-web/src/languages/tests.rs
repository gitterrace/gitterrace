use super::{Held, LocQuery, View, sample, shown};
use crate::languages::chain::first_from;
use gitterrace_index::Period;

#[test]
fn range_is_clamped_and_ordered() {
    let q = LocQuery { from: Some(50), to: Some(7), ..LocQuery::default() };
    assert_eq!(View::parse(&q, None, 100, Period::All).range, Some((7, 7)));
    let q = LocQuery { from: Some(3), to: Some(999), ..LocQuery::default() };
    assert_eq!(View::parse(&q, None, 100, Period::All).range, Some((3, 99)));
    assert_eq!(View::parse(&LocQuery::default(), None, 100, Period::All).range, None);
}

#[test]
fn sampling_keeps_the_ends_of_the_range() {
    let s = sample(0, 9_999);
    assert!(s.len() <= 700);
    assert_eq!((s[0], *s.last().unwrap_or(&0)), (0, 9_999));
    let r = sample(1234, 5678);
    assert_eq!((r[0], *r.last().unwrap_or(&0)), (1234, 5678));
    assert_eq!(sample(0, 4), vec![0, 1, 2, 3, 4]);
}

/// The page reads a bounded set of subjects whatever the history length, so a cold
/// load never walks every commit message.
#[test]
fn subjects_fetched_are_bounded_by_what_the_page_shows() {
    let view = View {
        time: false,
        share: false,
        range: None,
        picked: false,
        period: Period::All,
        dflt: Period::All,
        held: Held::Commits,
    };
    assert!(shown(400_000, view).len() <= 702 + 500);
    let ranged = View { range: Some((10, 399_990)), ..view };
    let v = shown(400_000, ranged);
    assert!(v.len() <= 704 + 500);
    assert!(v.contains(&10) && v.contains(&399_990) && v.contains(&399_500));
}

#[test]
fn links_keep_state() {
    let v = View {
        time: true,
        share: false,
        range: Some((1, 2)),
        picked: true,
        period: Period::All,
        dflt: Period::All,
        held: Held::Commits,
    };
    assert_eq!(v.href(|v| v.share = true), "/stats/languages?x=time&scale=share&from=1&to=2");
}

/// Days listed newest first: position 4 is the oldest, 0 the newest.
#[test]
fn stretch_starts_at_the_first_position_dated_in_the_period() {
    // Oldest to newest: 10, 11, 20, 12 (a later commit with an older author date), 21.
    let newest_first = [21, 12, 20, 11, 10];
    // Period starts on day 12: position 2 (day 20) is the first in it, and the later
    // commit dated 12 is included though it is older than 20.
    assert_eq!(first_from(newest_first.into_iter(), 5, 12), Some(2));
    // Period starts on day 11: position 1.
    assert_eq!(first_from(newest_first.into_iter(), 5, 11), Some(1));
    // A period that starts on day 13 still begins at position 2, and the commit dated 12
    // after it stays inside the stretch.
    assert_eq!(first_from(newest_first.into_iter(), 5, 13), Some(2));
    // Nothing is dated that late.
    assert_eq!(first_from(newest_first.into_iter(), 5, 22), None);
    // The whole chain.
    assert_eq!(first_from(newest_first.into_iter(), 5, 0), Some(0));
}

#[test]
fn period_links_replace_a_dragged_range_and_the_default_stays_out() {
    let v = View {
        time: false,
        share: false,
        range: Some((1, 2)),
        picked: true,
        period: Period::All,
        dflt: Period::All,
        held: Held::Commits,
    };
    assert_eq!(
        v.href(|v| {
            v.period = Period::Month;
            v.picked = false;
            v.range = None;
        }),
        "/stats/languages?q=after%3A1m"
    );
    assert_eq!(
        v.href(|v| {
            v.picked = false;
            v.range = None;
        }),
        "/stats/languages"
    );
}
