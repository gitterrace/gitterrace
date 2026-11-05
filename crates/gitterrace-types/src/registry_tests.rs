//! Every string-encoded enum is covered by a codec round-trip test.

use std::fs;
use std::path::Path;

/// Names declared through `string_enum!` in `text`.
fn declared(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("string_enum! {") {
        rest = &rest[i + "string_enum! {".len()..];
        let Some(j) = rest.find("pub enum ") else {
            break;
        };
        let name: String = rest[j + 9..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        out.push(name);
    }
    out
}

fn sources() -> String {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut all = String::new();
    for e in fs::read_dir(dir).expect("src") {
        let p = e.expect("entry").path();
        if p.file_name()
            .is_some_and(|n| n == "codec.rs" || n == "registry_tests.rs")
        {
            continue;
        }
        all.push_str(&fs::read_to_string(p).expect("read"));
    }
    all
}

fn uncovered(text: &str) -> Vec<String> {
    declared(text)
        .into_iter()
        .filter(|n| !text.contains(&format!("assert_round_trip::<{n}>()")))
        .collect()
}

#[test]
fn every_string_enum_has_a_round_trip_test() {
    let text = sources();
    assert!(declared(&text).len() >= 10, "the scan found the enums");
    assert_eq!(uncovered(&text), Vec::<String>::new());
}

#[test]
fn an_enum_without_a_round_trip_test_is_reported() {
    let planted = "string_enum! {\n    pub enum Planted {\n        A => \"a\",\n    }\n}\n";
    assert_eq!(uncovered(planted), ["Planted"]);
    let covered = format!("{planted}\nassert_round_trip::<Planted>();");
    assert!(uncovered(&covered).is_empty());
}
