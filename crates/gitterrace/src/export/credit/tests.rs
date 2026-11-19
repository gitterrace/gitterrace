use std::cell::Cell;

use gitterrace_testkit::Fixture;

use super::{Credit, add_footer, public_https};

const MIT: &str = "MIT License\n\nCopyright (c) 2024 A\n\nPermission is hereby granted, free of \
    charge, to any person obtaining a copy\nof this software\n";

thread_local! {
    /// License blobs whose content was read on this thread.
    pub(super) static LICENSE_READS: Cell<usize> = const { Cell::new(0) };
}

fn credit_for(
    dir: &std::path::Path,
    license: Option<(&str, &str)>,
    remote: Option<&str>,
) -> String {
    let mut f = Fixture::init(dir).write("a.txt", "x");
    if let Some((name, body)) = license {
        f = f.write(name, body);
    }
    let f = f.commit("c");
    if let Some(url) = remote {
        f.git(&["remote", "add", "origin", url]);
    }
    let repo = gitterrace_git::Repo::discover(dir).expect("repo");
    let head = repo.resolve("HEAD").expect("head");
    Credit::read(&repo, &head, &repo.name(), None).footer("")
}

#[test]
fn mit_license_and_origin_link() {
    let d = tempfile::tempdir().expect("tempdir");
    let line = credit_for(
        &d.path().join("proj"),
        Some(("LICENSE", MIT)),
        Some("git@github.com:owner/proj.git"),
    );
    assert!(line.contains("Source: proj, MIT license, "), "{line}");
    assert!(
        line.contains("<a href=\"https://github.com/owner/proj\">"),
        "{line}"
    );
}

#[test]
fn credentials_are_stripped() {
    let d = tempfile::tempdir().expect("tempdir");
    let line = credit_for(
        &d.path().join("proj"),
        None,
        Some("https://user:secret@github.com/owner/proj.git"),
    );
    assert!(!line.contains("secret") && !line.contains("user"), "{line}");
    assert!(line.contains("https://github.com/owner/proj"), "{line}");
}

#[test]
fn no_remote_and_no_license_gives_a_bare_line() {
    let d = tempfile::tempdir().expect("tempdir");
    let line = credit_for(&d.path().join("proj"), None, None);
    assert!(line.contains(">Source: proj.</div>"), "{line}");
    assert!(!line.contains("license") && !line.contains("<a "), "{line}");
}

#[test]
fn unknown_license_links_the_file() {
    let d = tempfile::tempdir().expect("tempdir");
    let line = credit_for(&d.path().join("proj"), Some(("LICENSE.txt", "mine")), None);
    assert!(
        line.contains("<a href=\"/blob/HEAD/LICENSE.txt\">license</a>"),
        "{line}"
    );
}

// Windows refuses `<` and `>` in a folder name, so no repository there can be named this way.
#[cfg(not(windows))]
#[test]
fn names_are_escaped() {
    let d = tempfile::tempdir().expect("tempdir");
    let line = credit_for(&d.path().join("<script>x"), None, None);
    assert!(!line.contains("<script>"), "{line}");
    assert!(line.contains("&lt;script&gt;x"), "{line}");
}

#[test]
fn only_public_hosts_become_links() {
    for bad in [
        "/home/me/repo",
        "file:///srv/repo",
        "http://example.com/a/b",
        "https://localhost/a/b",
        "https://192.168.1.5/a/b",
        "me@myserver:repo",
        "https://box.local/a/b",
    ] {
        assert_eq!(public_https(bad), None, "{bad}");
    }
    assert_eq!(
        public_https("ssh://git@codeberg.org:22/o/r.git").as_deref(),
        Some("https://codeberg.org/o/r")
    );
}

#[test]
fn footer_goes_before_the_closing_body() {
    let out = add_footer(b"<body>m</body></html>", "<footer>f</footer>");
    assert_eq!(out, b"<body>m<footer>f</footer></body></html>");
}

#[test]
fn oversized_license_is_never_read() {
    let d = tempfile::tempdir().expect("tempdir");
    let body = format!("{MIT}{}", " ".repeat(super::MAX_LICENSE + 1));
    LICENSE_READS.with(|n| n.set(0));
    let line = credit_for(&d.path().join("big"), Some(("LICENSE", &body)), None);
    assert_eq!(
        LICENSE_READS.with(Cell::get),
        0,
        "the oversized blob was read"
    );
    assert!(!line.contains("MIT"), "{line}");

    let small = tempfile::tempdir().expect("tempdir");
    let line = credit_for(&small.path().join("ok"), Some(("LICENSE", MIT)), None);
    assert_eq!(
        LICENSE_READS.with(Cell::get),
        1,
        "a normal license is read once"
    );
    assert!(line.contains("MIT"), "{line}");
}
