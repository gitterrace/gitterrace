//! The log's filter against the work it does: with the index, no keyword walks git; without
//! it, the walk stops at its cap; and the indexed and walked answers agree.

use std::sync::Arc;

use gitterrace_index::query::{Matcher, Query};
use gitterrace_testkit::Fixture;

use crate::app::App;
use crate::routes::log_query::{WALK_CAP, page};

fn app(p: &std::path::Path) -> Arc<App> {
    let repo = gitterrace_git::Repo::discover(p).expect("repo");
    let index = gitterrace_index::Index::open(&repo).expect("index");
    Arc::new(App::new(repo, index))
}

fn m(q: &str) -> Matcher {
    Matcher::new(&Query::parse(q).expect("query"))
}

fn ids(a: &App, q: &str) -> (Vec<String>, crate::routes::log_query::Work) {
    let lp = page(a, "HEAD", &m(q), 0).expect("page");
    (lp.list.into_iter().map(|c| c.subject).collect(), lp.work)
}

/// Commits by two authors over two directories, with conventional subjects.
fn history(dir: &std::path::Path) -> std::path::PathBuf {
    let mut f = Fixture::init(dir);
    for i in 0..12 {
        let (who, path, kind) = match i % 3 {
            0 => ("Ann <ann@x.io>", "src/a.rs", "fix"),
            1 => ("Bob <bob@x.io>", "docs/b.md", "feat"),
            _ => ("Ann <ann@x.io>", "docs/c.md", "chore"),
        };
        f = f.write(path, &format!("{i}\n"));
        f.git(&["add", "-A"]);
        f.git(&[
            "commit",
            "-q",
            "--author",
            who,
            "-m",
            &format!("{kind}: change {i} login"),
        ]);
    }
    f.path().to_path_buf()
}

const QUERIES: &[&str] = &[
    "author:ann@x.io",
    "author:Bob path:docs",
    "!author:Ann",
    "type:fix,feat",
    "path:src,docs !type:chore",
    "!path:docs",
    "\"change 1\"",
    "login !merge author:Ann",
];

#[test]
fn indexed_pages_walk_no_git_and_match_the_walk() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = history(dir.path());
    let cold = app(&p);
    let walked: Vec<_> = QUERIES.iter().map(|q| ids(&cold, q)).collect();
    for (q, (_, w)) in QUERIES.iter().zip(&walked) {
        assert!(w.walked > 0 && w.walked <= WALK_CAP, "{q}: {w:?}");
    }
    let warm = app(&p);
    warm.index
        .update(&warm.repo, "HEAD", &|_| {})
        .expect("update");
    warm.mark_ready();
    for (q, (want, _)) in QUERIES.iter().zip(&walked) {
        let (got, work) = ids(&warm, q);
        assert_eq!(&got, want, "{q}");
        assert!(!want.is_empty(), "{q} selects something");
        let words = m(q).words().len();
        // Only message words read git, and only for the candidates the index kept.
        assert!(
            words > 0 || work.walked == 0,
            "{q} walked git with the index ready: {work:?}"
        );
        assert!(work.walked <= 12 && work.rows <= 12, "{q}: {work:?}");
    }
}
