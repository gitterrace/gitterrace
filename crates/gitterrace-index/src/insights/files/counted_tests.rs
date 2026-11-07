//! The per-file reports against the counted set.

use super::tests::head;
use crate::insights::fixtures::{file, rec};
use crate::{Insights, Options, Period};

/// Every per-file report counts the commits [`crate::Counted`] counts, under every clause of
/// the selection and the options at once: a change to the membership rule reaches them all.
/// Hotspots reads the same touches as ownership, coupling and code age, so it stands for them:
/// per file, its changes are the counted commits that touch the file.
#[test]
fn per_file_reports_follow_the_counted_set() {
    use crate::feed::{FixedClock, MemorySource};
    use crate::{CountContext, Scope, ScopeParams};
    type Case = (&'static str, fn(&mut ScopeParams));
    let mut recs = vec![
        rec(
            "a",
            "Ann",
            0,
            vec![file("src/a.rs", 3, 0, 3), file("docs/a.md", 1, 0, 1)],
        ),
        rec(
            "b",
            "Bob",
            10,
            vec![file("src/a.rs", 1, 1, 3), file("src/b.rs", 2, 0, 2)],
        ),
        rec("m", "Ann", 20, vec![file("src/a.rs", 5, 0, 8)]),
        rec(
            "bot",
            "dependabot[bot]",
            30,
            vec![file("src/b.rs", 1, 1, 2)],
        ),
        rec(
            "c",
            "Ann",
            40,
            vec![file("src/a.rs", 1, 0, 9), file("src/b.rs", 1, 0, 3)],
        ),
    ];
    recs[1].kind = Some("fix".to_owned());
    recs[2].merge = true;
    let hf = head(&[("src/a.rs", 9), ("src/b.rs", 3), ("docs/a.md", 1)]);
    let src = MemorySource::new(&recs);
    let cx = CountContext::new(&src, &FixedClock(i64::MAX));
    let cases: [Case; 5] = [
        ("nothing", |_| {}),
        ("an author", |p| {
            p.select.author = Some("ann@x.io".to_owned());
        }),
        ("a type", |p| {
            p.select.kind = Some("fix".to_owned());
        }),
        ("a path", |p| {
            p.select.path = Some("src".to_owned());
        }),
        ("a range", |p| {
            p.from = Some(crate::insights::format_date(
                crate::insights::fixtures::MON / 86_400 + 5,
            ));
        }),
    ];
    for bots in [false, true] {
        let opts = Options {
            include_bots: bots,
            ..Options::default()
        };
        let ins = Insights::new(&recs, opts.clone(), None);
        for (what, set) in &cases {
            let mut p = ScopeParams::default();
            set(&mut p);
            let s = Scope::from_params(&p, opts.clone(), Period::All).expect("scope");
            let c = ins.counted(&s, cx);
            let path = s.selection().path();
            let mut want: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::default();
            for (_, rows) in c.rows() {
                for r in rows {
                    let name = ins
                        .source()
                        .path(r.path)
                        .map(|n| n.to_string())
                        .unwrap_or_default();
                    if path.is_none_or(|p| crate::under(&name, p)) {
                        *want.entry(name).or_default() += 1;
                    }
                }
            }
            let got: std::collections::BTreeMap<String, usize> = ins
                .hotspots_in(&hf, &c)
                .files
                .into_iter()
                .map(|f| (f.path, f.changes))
                .collect();
            assert_eq!(got, want, "hotspots, {what}, bots {bots}");
        }
    }
}
