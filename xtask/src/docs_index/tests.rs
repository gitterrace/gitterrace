use super::*;

/// The registry is checked both ways: a file nobody listed and a listing nobody wrote.
#[test]
fn a_listed_document_missing_on_disk_and_an_unlisted_one_both_fail() {
    let on_disk = vec!["docs/a.md".to_owned(), "docs/new.md".to_owned()];
    let bad = registry_problems(&on_disk, ["docs/a.md", "docs/gone.md"].into_iter());
    assert_eq!(bad.len(), 2, "{bad:?}");
    assert!(bad[0].contains("docs/new.md: not listed"), "{bad:?}");
    assert!(
        bad[1].contains("lists docs/gone.md, which does not exist"),
        "{bad:?}"
    );
    let listed = ["docs/a.md", "docs/new.md"];
    let none = registry_problems(&on_disk, listed.into_iter());
    assert_eq!(none, Vec::<String>::new());
}

/// The index and the checks read the same list: an unlisted page, a dead link, a missing
/// anchor, an unknown key and an unknown flag each go red.
#[test]
fn each_kind_of_drift_goes_red() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("docs")).expect("docs");
    for (p, ..) in DOCS {
        let file = root.join(p);
        std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
        std::fs::write(&file, "# Title\n\n## Part two\n").expect("write");
    }
    std::fs::write(root.join(INDEX), "# Docs\n").expect("index");
    let reverse = |l: &String| l.contains("no row for") || l.contains("leaves out");
    let clean = check(root).expect("check");
    assert!(clean.iter().all(reverse), "{clean:?}");
    assert!(
        clean.iter().any(|l| l.contains("no row for `server.addr`")),
        "{clean:?}"
    );
    assert!(
        clean.iter().any(|l| l.contains("leaves out `--public`")),
        "{clean:?}"
    );

    std::fs::write(root.join("docs/stray.md"), "# Stray\n").expect("stray");
    std::fs::write(
        root.join("docs/why.md"),
        "# Why\n\n[a](gone.md) [b](api.md#part-two) [c](api.md#part-3) [d](#why)\n\n\
         Set `server.public = true`, `server.addr`, `index.html`, `--public`, `--bogus`.\n\n\
         ```toml\n[server]\nis_public = true\nlive = 3\n```\n\n\
         ```sh\ngit terrace export --commits 3 --pages 9\n```\n\n\
         See `scripts/gone.sh` and `scripts/` and `docs/why.md`.\n",
    )
    .expect("why");
    let bad: Vec<String> = check(root)
        .expect("check")
        .into_iter()
        .filter(|l| !reverse(l))
        .collect();
    let bad = bad.join("\n");
    for needle in [
        "docs/stray.md: not listed",
        "gone.md does not exist",
        "no heading #part-3",
        "`server.public` is not a config key",
        "`--bogus` is not a flag",
        "`server.live` in a toml example",
        "`--pages` is not a flag",
        "`scripts/gone.sh` does not exist",
    ] {
        assert!(bad.contains(needle), "missing {needle:?} in:\n{bad}");
    }
    for fine in [
        "part-two",
        "#why",
        "server.addr",
        "index.html",
        "--public",
        "--commits",
        "`docs/why.md` does not",
    ] {
        assert!(!bad.contains(fine), "{fine} flagged:\n{bad}");
    }
}

#[test]
fn anchors_follow_the_page_renderer() {
    let a = anchors("# A `[server]` key\n\n## Same\n\n## Same\n\n### 2.1 Byte layout\n");
    for want in ["a-server-key", "same", "same-1", "21-byte-layout"] {
        assert!(a.contains(want), "{want} in {a:?}");
    }
}

#[test]
fn a_command_fence_marked_text_is_refused() {
    let run = "```text\ngit terrace serve [PATHS]...\n```\n";
    assert_eq!(plain_commands("d.md", run).len(), 1, "{run}");
    let sh = "```sh\ngit terrace serve [PATHS]...\n```\n";
    assert!(plain_commands("d.md", sh).is_empty(), "{sh}");
    let table = "```text\ngit terrace serve   Serve the repository\n```\n";
    assert!(plain_commands("d.md", table).is_empty(), "{table}");
    let line = "```text\nsource: <repo>@<sha>\n```\n";
    assert!(plain_commands("d.md", line).is_empty(), "{line}");
}

#[test]
fn a_hand_list_against_a_generated_list_is_refused() {
    let two = "## S\n\n- hand\n\n<!-- BEGIN GENERATED: x -->\n- gen\n\
               <!-- END GENERATED: x -->\n";
    assert_eq!(split_lists("d.md", two).len(), 1, "{two}");
    let wrapped = "- hand item\n  wrapped on\n\n<!-- BEGIN GENERATED: x -->\n- gen\n";
    assert_eq!(split_lists("d.md", wrapped).len(), 1, "{wrapped}");
    let after = "<!-- BEGIN GENERATED: x -->\n- gen\n<!-- END GENERATED: x -->\n\n- hand\n";
    assert_eq!(split_lists("d.md", after).len(), 1, "{after}");
    let one = "## S\n\nProse.\n\n<!-- BEGIN GENERATED: x -->\n- gen\n\
               <!-- END GENERATED: x -->\n";
    assert!(split_lists("d.md", one).is_empty(), "{one}");
}
