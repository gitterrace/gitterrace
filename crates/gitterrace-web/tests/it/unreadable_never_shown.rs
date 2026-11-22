//! A cache whose blocks fail their check halfway through a page is never shown as data. Every
//! insights page and every insights API answer over it is a 503, the indexing page while the
//! cache is rebuilt, or exactly what a clean cache answers once the rebuild is done. Never a
//! page of zeros: a read error that folded into absence rendered one, with status 200, and
//! memoized it.
#![expect(clippy::expect_used, reason = "test setup")]

use std::collections::BTreeMap;
use std::path::Path;

use gitterrace_testkit::oracle::{self, normalize};
use gitterrace_testkit::{Fixture, page};

fn targets() -> Vec<String> {
    let names: Vec<&str> = gitterrace_api::insight_list().map(|(n, _)| n).collect();
    oracle::targets(&gitterrace_web::WARM_PAGES, &names)
}

fn repo(dir: &Path) -> std::path::PathBuf {
    let mut f = Fixture::init(&dir.join("r")).at("2024-02-01T10:00:00Z");
    for i in 0..12 {
        f = f
            .write(&format!("src/f{}.rs", i % 4), &format!("fn f() {{ {i} }}\n"))
            .commit(&format!("feat(core): change {i}"));
    }
    f.path().to_owned()
}

/// Flips one byte inside every commit, row and aggregate block of every segment. The set still
/// opens (its indexes and strings are intact): a page meets the failed check only once it reads
/// history, halfway through its build.
fn corrupt(cache: &Path) {
    use gitterrace_index::seg::format::section;
    let store = gitterrace_index::seg::read::Store::open(cache).expect("open").expect("a set");
    let mut flips: Vec<(String, u64)> = Vec::new();
    for c in &store.segments {
        for sec in [section::COMMITS, section::ROWS, section::AUTHOR_DAY, section::PATH_AUTHOR] {
            for e in c.blocks(sec) {
                flips.push((c.name().to_owned(), e.offset + u64::from(e.len / 2)));
            }
        }
    }
    drop(store);
    assert!(!flips.is_empty(), "the set has aggregate blocks to corrupt");
    for (name, at) in flips {
        let p = cache.join(name);
        let mut b = std::fs::read(&p).expect("read");
        let i = usize::try_from(at).expect("offset");
        b[i] ^= 0x5a;
        std::fs::write(&p, &b).expect("write");
    }
}

/// Every target's answer from a fresh server over `repo`, once it is ready. Fresh, so no block
/// read before is still in memory.
async fn answers(repo: &Path) -> BTreeMap<String, (u16, String)> {
    let router = gitterrace_web::app_router(repo).expect("router");
    oracle::ready(&router).await;
    let mut out = BTreeMap::new();
    for t in targets() {
        let (status, body) = page(&router, &t).await;
        out.insert(t, (status, normalize(&body)));
    }
    out
}

/// `target`'s answer from a fresh server over a fresh copy of `repo` whose cache is corrupt:
/// the target is the first to read it, so an earlier target's rebuild cannot hide its own
/// defect, and no earlier server still writes into the folder.
async fn first_answer(repo: &Path, at: &Path, target: &str) -> (u16, String) {
    let copy = at.join("r");
    copy_dir(repo, &copy);
    corrupt(&copy.join(".git/gitterrace"));
    let router = gitterrace_web::app_router(&copy).expect("router");
    oracle::ready(&router).await;
    let (status, body) = page(&router, target).await;
    (status, normalize(&body))
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for e in std::fs::read_dir(from).expect("ls").flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().expect("type").is_dir() {
            copy_dir(&src, &dst);
        } else if !e.file_name().to_string_lossy().ends_with(".tmp") {
            std::fs::copy(&src, &dst).expect("copy");
        }
    }
}

#[test]
fn a_cache_that_fails_a_read_is_never_shown_as_data() {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let dir = tempfile::tempdir().expect("tmp");
    let repo = repo(dir.path());
    let clean = rt.block_on(async {
        let _router = crate::common::ready_router(&repo).await;
        answers(&repo).await
    });
    for (t, (status, body)) in &clean {
        assert_eq!(*status, 200, "{t} answers on a clean cache");
        assert!(!body.contains("data-progress"), "{t} shows data on a clean cache");
    }
    let mut shown = Vec::new();
    for t in targets() {
        let at = tempfile::tempdir().expect("tmp");
        let (status, body) = rt.block_on(first_answer(&repo, at.path(), &t));
        // The overview needs no index: over a bad cache it shows its code and leaves the
        // insights sidebar out, which is a refusal of the data, not data.
        let no_side = t == "/" && !body.contains("<aside>");
        let refused = status == 503 || body.contains("data-progress") || no_side;
        if !refused && clean.get(&t).is_none_or(|(_, want)| *want != body) {
            shown.push(format!("{t}: status {status}"));
        }
    }
    assert!(shown.is_empty(), "answers built over an unreadable cache were shown: {shown:#?}");
}
