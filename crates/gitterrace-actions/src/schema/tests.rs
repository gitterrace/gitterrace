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
    }
}

fn keys_of(p: &Payload) -> BTreeSet<String> {
    let v = serde_json::to_value(p).expect("json");
    v.as_object().expect("object").keys().cloned().collect()
}

#[test]
fn payload_table_is_the_payload() {
    assert_eq!(keys_of(&payload(true)), names(&PAYLOAD_FIELDS));
    let absent_when_unset: BTreeSet<String> =
        names(&PAYLOAD_FIELDS).difference(&keys_of(&payload(false))).cloned().collect();
    let optional: BTreeSet<String> =
        PAYLOAD_FIELDS.iter().filter(|f| !f.required).map(|f| f.name.to_owned()).collect();
    assert_eq!(absent_when_unset, optional);
}

#[test]
fn env_table_is_the_environment() {
    let env: BTreeSet<String> = payload(true).env().keys().map(|k| (*k).to_owned()).collect();
    let doc: BTreeSet<String> = ENV_VARS.iter().map(|(k, _)| (*k).to_owned()).collect();
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
