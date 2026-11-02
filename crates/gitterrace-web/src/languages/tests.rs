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

/// The page reads a bounded set of subjects whatever the history length, so a cold
/// load never walks every commit message.
#[test]
fn subjects_fetched_are_bounded_by_what_the_page_shows() {
    let ch = super::Chain {
        ids: vec![String::new(); 400_000],
        times: Vec::new(),
        offsets: Vec::new(),
        adds: Vec::new(),
        dels: Vec::new(),
        langs: Vec::new(),
        lines: Vec::new(),
    };
    let view = View {
        time: false,
        share: false,
        range: None,
    };
    assert!(ch.shown(view).len() <= 702 + 500);
    let ranged = View {
        range: Some((10, 399_990)),
        ..view
    };
    let v = ch.shown(ranged);
    assert!(v.len() <= 704 + 500);
    assert!(v.contains(&10) && v.contains(&399_990) && v.contains(&399_500));
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
