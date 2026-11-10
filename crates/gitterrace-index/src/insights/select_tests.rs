use std::sync::Arc;

use super::{OptionParams, Scoped, Selection, under};
use crate::feed::{CommitRange, InsightsSource, MemorySource, Rank};
use crate::insights::fixtures::{file, rec};
use crate::query::Matcher;
use crate::{CommitRecord, Options};

fn sample() -> Vec<CommitRecord> {
    let mut v = vec![
        rec("a", "ann", 1, vec![file("crates/web/a.rs", 3, 0, 3)]),
        rec("b", "bob", 50, vec![file("crates/git/b.rs", 3, 0, 3)]),
        rec("c", "ann", 99, vec![file("crates/web/c.rs", 3, 0, 3)]),
        rec("d", "ann", 100, vec![file("crates/webby/d.rs", 3, 0, 3)]),
    ];
    for (r, (k, s)) in v.iter_mut().zip([
        (Some("fix"), Some("web")),
        (Some("feat"), None),
        (None, None),
        (Some("fix"), Some("git")),
    ]) {
        r.kind = k.map(Into::into);
        r.scope = s.map(Into::into);
        r.files.push(file("other/x.rs", 1, 0, 1));
    }
    v
}

fn sel(q: &str) -> Selection {
    Selection {
        q: Some(q.to_owned()),
        ..Selection::default()
    }
}

fn matcher(s: &Selection) -> Matcher {
    Matcher::new(&s.query().expect("query"))
}

fn ids(s: &Selection) -> Vec<String> {
    matcher(s)
        .apply(&sample(), i64::MAX)
        .iter()
        .map(|r| r.id.clone())
        .collect()
}

#[test]
fn selections_stack() {
    assert_eq!(ids(&Selection::default()), ["a", "b", "c", "d"]);
    assert_eq!(ids(&sel("type:fix")), ["a", "d"]);
    assert_eq!(ids(&sel("type:fix scope:web")), ["a"]);
    assert_eq!(ids(&sel("type:other")), ["c"]);
    assert_eq!(ids(&sel("scope:none")), ["b", "c"]);
    assert_eq!(ids(&sel("type:fix author:ANN@x.io")), ["a", "d"]);
    assert_eq!(ids(&sel("  ")), ["a", "b", "c", "d"]);
    assert_eq!(ids(&sel("type:fix,feat")), ["a", "b", "d"]);
    assert_eq!(ids(&sel("!type:fix,feat")), ["c"]);
    assert_eq!(ids(&sel("!author:bob")), ["a", "c", "d"]);
    assert_eq!(ids(&sel("path:crates/git,crates/webby")), ["b", "d"]);
    assert_eq!(ids(&sel("!path:other")), ["a", "b", "c", "d"]);
}

/// Every one-key form of an older link selects what its `q` form selects.
#[test]
fn one_key_forms_fold_into_the_query() {
    let one = |k: &str, v: &str| {
        let v = Some(v.to_owned());
        let mut s = Selection::default();
        match k {
            "type" => s.kind = v,
            "scope" => s.scope = v,
            "author" => s.author = v,
            _ => s.path = v,
        }
        s
    };
    for (k, v, q) in [
        ("type", "fix", "type:fix"),
        ("scope", "none", "scope:none"),
        ("author", "Ann Lee", "author:\"Ann Lee\""),
        ("path", "/crates/web/", "path:crates/web"),
        ("path", "/", ""),
    ] {
        let s = one(k, v);
        assert_eq!(
            s.query().map(|q| q.to_string()).as_deref(),
            Ok(q),
            "{k}={v}"
        );
        assert_eq!(ids(&s), ids(&sel(q)), "{k}={v}");
    }
    let both = Selection {
        author: Some("ann".into()),
        ..sel("type:fix")
    };
    assert_eq!(
        both.query().map(|q| q.to_string()).as_deref(),
        Ok("author:ann type:fix")
    );
    assert!(sel("a | b").query().is_err());
}

#[test]
fn path_keeps_only_files_under_the_prefix() {
    let out = matcher(&sel("path:/crates/web/")).apply(&sample(), i64::MAX);
    assert_eq!(
        out.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["a", "c"]
    );
    assert!(out.iter().all(|r| r.files.len() == 1));
    assert!(under("crates/web/a.rs", "crates/web") && !under("crates/webby/d.rs", "crates/web"));
}

#[test]
fn url_options_override_the_defaults() {
    let defaults = Options {
        exclude: vec!["docs/*".into()],
        include_bots: true,
        ..Options::default()
    };
    let p = OptionParams {
        bots: Some("0".into()),
        ignore: Some("0".into()),
        exclude: Some(" *.md, ,x/*".into()),
        ..OptionParams::default()
    };
    let o = p.apply(defaults.clone());
    assert!(!o.include_bots && !o.respect_ignore && !o.include_generated);
    assert_eq!(o.exclude, ["docs/*", "*.md", "x/*"]);
    assert_eq!(OptionParams::default().apply(defaults.clone()), defaults);
}

/// The scoped view must keep exactly the commits and rows `Matcher::apply` plus the
/// window keep, in order, with sums over the kept rows. Every selection of
/// `selections_stack` plus path and window cells.
#[test]
fn scoped_equals_apply() {
    let recs = sample();
    let base: Arc<dyn InsightsSource> = Arc::new(MemorySource::shared(Arc::new(recs.clone())));
    let cells = [
        Selection::default(),
        sel("type:fix"),
        sel("type:fix scope:web"),
        sel("type:other"),
        sel("scope:none"),
        sel("type:fix author:ANN@x.io"),
        sel("author:bob"),
        sel("path:/crates/web/"),
        sel("!path:crates/web,other"),
        sel("!author:bob type:fix,other"),
    ];
    let t = |d: i64| crate::insights::fixtures::MON + d * crate::insights::fixtures::DAYS;
    for s in &cells {
        for w in [None, Some(t(40)..t(100)), Some(t(0)..t(1))] {
            let now = crate::feed::reference_time(&*base);
            let want: Vec<_> = matcher(s)
                .apply(&recs, now)
                .iter()
                .filter(|r| w.as_ref().is_none_or(|w| w.contains(&r.time)))
                .cloned()
                .collect();
            let v = Scoped::new(&base, &matcher(s), w.as_ref());
            assert_eq!(v.len(), want.len(), "{s:?} {w:?}");
            let rows: Vec<_> = v.commits_in(CommitRange::All).collect();
            for (i, (c, r)) in rows.iter().zip(&want).enumerate() {
                assert_eq!(c.rank, Rank(u32::try_from(i).unwrap_or(0)));
                assert_eq!(v.commit_id(c.rank).as_deref(), Some(r.id.as_str()));
                assert_eq!(c.files as usize, r.files.len(), "{s:?} {}", r.id);
                let add: u64 = r.files.iter().map(|f| u64::from(f.additions)).sum();
                assert_eq!(c.sums.add, add);
                assert_eq!(v.find_commit(&r.id), Ok(Some(c.rank)));
            }
            assert_eq!(v.len(), want.len());
        }
    }
}
