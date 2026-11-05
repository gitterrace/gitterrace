//! A setting that documents a fixed set of choices is an enum, never a plain string.

use crate::{Key, Reload, keys, parse};

/// Whether the doc lists choices as `a, b or c`.
fn lists_choices(doc: &str) -> bool {
    let words: Vec<&str> = doc.split_whitespace().collect();
    words
        .windows(3)
        .any(|w| w[0].ends_with(',') && (w[1] == "or" || w[2] == "or"))
}

/// Whether the loader takes a value that is no choice at all.
fn accepts_anything(k: &Key) -> bool {
    let text = format!("[{}]\n{} = \"zz-no-such-choice\"\n", k.section, k.name);
    parse(&text).is_ok()
}

/// Ids of the string settings that list choices yet take any text.
fn untyped_choices(all: &[Key], accepts: fn(&Key) -> bool) -> Vec<&'static str> {
    all.iter()
        .filter(|k| k.ty == "string" && lists_choices(k.doc) && accepts(k))
        .map(|k| k.id)
        .collect()
}

#[test]
fn keys_that_document_choices_reject_other_text() {
    assert_eq!(
        untyped_choices(&keys(), accepts_anything),
        Vec::<&str>::new()
    );
}

#[test]
fn a_plain_string_that_lists_choices_is_reported() {
    let planted = Key {
        section: "server",
        name: "addr",
        id: "server.addr",
        ty: "string",
        default: String::new(),
        doc: "Pick one: fast, slow or off.",
        reload: Reload::Live,
        cost: false,
    };
    assert_eq!(untyped_choices(&[planted], |_| true), ["server.addr"]);
    let all = keys();
    let theme = all.iter().find(|k| k.id == "display.theme").expect("key");
    assert!(lists_choices(theme.doc) || !accepts_anything(theme));
    assert!(!accepts_anything(theme));
}
