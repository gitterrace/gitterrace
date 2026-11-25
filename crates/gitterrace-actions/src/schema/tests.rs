use std::collections::BTreeSet;
use std::path::Path;

use crate::config::{Config, Kind};
use crate::payload::{CommitInfo, Lines, Payload, RepoInfo};
use crate::schema::{
    ACTION_KEYS, ENV_VARS, FILE_KEYS, Item, KINDS, PAYLOAD_FIELDS, PLACEHOLDERS, SCRIPT_KEYS,
    SEND_KEYS, SEND_PLACEHOLDERS,
};

fn names(items: &[Item]) -> BTreeSet<String> {
    items.iter().map(|i| i.name.to_owned()).collect()
}

/// Names a serde or toml "expected one of `a`, `b`" message lists.
fn expected(message: &str) -> BTreeSet<String> {
    let tail = message.split("expected one of").nth(1).unwrap_or("");
    tail.split('`').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, s)| s.to_owned()).collect()
}

fn payload(full: bool) -> Payload {
    Payload {
        v: 1,
        action: "a".into(),
        kind: Kind::Line,
        repo: RepoInfo { name: "r".into(), root: "/r".into(), id: Some("abc".into()) },
        rev: "HEAD".into(),
        sha: "0".repeat(40),
        path: full.then(|| "p".into()),
        lines: full.then_some(Lines { start: 1, end: 2 }),
        text: full.then(|| "t".into()),
        text_truncated: full.then_some(false),
        query: full.then(|| "q".into()),
        permalink: "u".into(),
        title: "t".into(),
        commit: full.then(|| CommitInfo {
            subject: "s".into(),
            author: "a".into(),
            date: "d".into(),
        }),
        markdown: "m".into(),
        vars: std::collections::BTreeMap::new(),
        findings: Vec::new(),
    }
}

fn keys_of(p: &Payload) -> BTreeSet<String> {
    let v = serde_json::to_value(p).expect("json");
    v.as_object().expect("object").keys().cloned().collect()
}

/// The fields every single-kind payload can carry: the table less the review-only one.
fn without_review(doc: &BTreeSet<String>) -> BTreeSet<String> {
    doc.iter().filter(|n| *n != "findings").cloned().collect()
}

#[test]
fn payload_table_is_the_payload() {
    let doc = names(&PAYLOAD_FIELDS);
    assert_eq!(keys_of(&payload(true)), without_review(&doc));
    let absent_when_unset: BTreeSet<String> =
        without_review(&doc).difference(&keys_of(&payload(false))).cloned().collect();
    let optional: BTreeSet<String> = PAYLOAD_FIELDS
        .iter()
        .filter(|f| !f.required && f.name != "findings")
        .map(|f| f.name.to_owned())
        .collect();
    assert_eq!(absent_when_unset, optional);
}

/// `findings` is documented as optional but only a review carries it: a single-kind payload
/// never has the key, which `payload_table_is_the_payload` cannot see.
#[test]
fn findings_is_the_one_documented_field_a_single_kind_never_has() {
    let all = names(&PAYLOAD_FIELDS);
    let mut have = keys_of(&payload(true));
    assert_eq!(all.difference(&have).collect::<Vec<_>>(), ["findings"]);
    have.insert("findings".to_owned());
    assert_eq!(have, all);
}

#[test]
fn env_table_is_the_environment() {
    let env: BTreeSet<String> = payload(true).env().keys().map(|k| (*k).to_owned()).collect();
    let mut doc: BTreeSet<String> = ENV_VARS.iter().map(|(k, _)| (*k).to_owned()).collect();
    // Set only for a review, which the single-kind fixture is not.
    assert!(doc.remove("GITTERRACE_FINDINGS"));
    assert_eq!(env, doc);
}

#[test]
fn kinds_table_is_the_kind_enum() {
    let doc: BTreeSet<String> =
        KINDS.iter().map(|(k, _)| crate::payload::kind_name(*k).to_owned()).collect();
    let err = serde_json::from_str::<Kind>("\"nope\"").expect_err("unknown kind");
    assert_eq!(doc, expected(&err.to_string()));
    assert_eq!(doc.len(), KINDS.len(), "a kind is listed twice");
}

fn parse_error(toml: &str) -> String {
    match Config::parse(toml, Path::new("/")) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    }
}

#[test]
fn key_tables_are_the_config_keys() {
    let top = parse_error("version = 1\nzzz = 1\n");
    assert_eq!(expected(&top), names(&FILE_KEYS), "{top}");
    let script =
        parse_error("version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/bin/echo\"]\nzzz = 1\n");
    assert_eq!(expected(&script), names(&SCRIPT_KEYS), "{script}");
    let action = parse_error(
        "version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\n\
             command = [\"/bin/echo\"]\nzzz = 1\n",
    );
    assert_eq!(expected(&action), names(&ACTION_KEYS), "{action}");
    let send = parse_error("version = 1\n[[send]]\nid = \"s\"\nzzz = 1\n");
    assert_eq!(expected(&send), names(&SEND_KEYS), "{send}");
}

#[test]
fn placeholder_table_is_the_placeholders() {
    let p = payload(true);
    for (name, _, _) in &PLACEHOLDERS[..11] {
        assert!(p.placeholder(name).is_some(), "{name}");
    }
    let doc: Vec<&str> = PLACEHOLDERS[..11].iter().map(|(n, _, _)| *n).collect();
    assert_eq!(doc, crate::config::FIELDS);
    let repo: Vec<&str> = PLACEHOLDERS.iter().filter(|p| p.1).map(|p| p.0).collect();
    assert_eq!(repo, crate::config::REPO_FIELDS);
}

#[test]
fn send_placeholder_table_is_the_fields() {
    let doc: Vec<&str> = SEND_PLACEHOLDERS[..7].iter().map(|(n, _)| *n).collect();
    assert_eq!(doc, crate::send::SEND_FIELDS);
}

/// The limits the tables state are the limits the loader enforces.
#[test]
fn stated_limits_hold() {
    // Disabled, so the program need not exist: Windows has no /bin/echo, and the limits are
    // checked on a disabled action all the same.
    let head = "version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\n\
                    enabled = false\ncommand = [\"/bin/echo\"]\n";
    assert!(Config::parse(&format!("{head}timeout = \"120s\"\n"), Path::new("/")).is_ok());
    assert!(Config::parse(&format!("{head}timeout = \"121s\"\n"), Path::new("/")).is_err());
    let vars = |n: usize| {
        let mut rows = String::new();
        for i in 0..n {
            rows += &["v", &i.to_string(), " = \"x\"\n"].concat();
        }
        format!("{head}[action.vars]\n{rows}")
    };
    assert!(Config::parse(&vars(32), Path::new("/")).is_ok());
    assert!(Config::parse(&vars(33), Path::new("/")).is_err());
    let long = format!("{head}[action.vars]\nv = \"{}\"\n", "x".repeat(4097));
    assert!(Config::parse(&long, Path::new("/")).is_err());
}

/// The reserved keys the `key` row names are the keys the loader refuses, split as the loader
/// splits them into the menu's and the page's. A key added to or dropped from `RESERVED_KEYS`
/// fails here until the row says so.
#[test]
fn reserved_keys_named_in_the_key_row_are_the_reserved_keys() {
    let doc = ACTION_KEYS.iter().find(|i| i.name == "key").expect("key row").doc;
    let named = |after: &str| -> Vec<char> {
        let part = doc.split(after).nth(1).and_then(|r| r.split(')').next()).expect(after);
        let mut keys: Vec<char> =
            part.split('`').skip(1).step_by(2).filter_map(|k| k.chars().next()).collect();
        keys.sort_unstable();
        keys
    };
    let reserved = |menu: bool| -> Vec<char> {
        let mut keys: Vec<char> = crate::config::RESERVED_KEYS
            .iter()
            .filter(|(_, who)| who.starts_with("the menu's") == menu)
            .map(|(k, _)| *k)
            .collect();
        keys.sort_unstable();
        keys
    };
    assert_eq!(named("Not a menu key ("), reserved(true), "menu keys in the key row");
    assert_eq!(named("or a page key ("), reserved(false), "page keys in the key row");
}
