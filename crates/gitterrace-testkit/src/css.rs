//! Dead-CSS check shared by the app and the site: a class selector that no source mentions.

use std::collections::BTreeSet;

/// Every class named in a selector of `css`, comments and declarations skipped.
#[must_use]
pub fn classes(css: &str) -> BTreeSet<String> {
    let mut plain = String::new();
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        plain.push_str(&rest[..i]);
        rest = rest[i + 2..].split_once("*/").map_or("", |(_, r)| r);
    }
    plain.push_str(rest);
    let mut found = BTreeSet::new();
    let mut head = String::new();
    for ch in plain.chars() {
        match ch {
            '{' => {
                for part in head.split('.').skip(1) {
                    let name: String = part
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                        .collect();
                    if !name.is_empty() && !part.starts_with(|c: char| c.is_ascii_digit()) {
                        found.insert(name);
                    }
                }
                head.clear();
            }
            '}' | ';' => head.clear(),
            _ => head.push(ch),
        }
    }
    found
}

fn is_ramp_member(name: &str, ramps: &[&str]) -> bool {
    let digits = name.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let prefix = &name[..name.len() - digits.len()];
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && ramps.contains(&prefix)
}

fn mentioned(haystack: &str, name: &str) -> bool {
    haystack.match_indices(name).any(|(i, _)| {
        let before = haystack[..i].chars().next_back();
        let after = haystack[i + name.len()..].chars().next();
        let word =
            |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        !word(before) && !word(after)
    })
}

/// The classes of `css` that `code` never mentions as a whole word. `ramps` are families built
/// as `format!("w{n}")`, checked by prefix. `prefixes` are classes written by a renderer.
#[must_use]
pub fn dead_classes(css: &str, code: &str, ramps: &[&str], prefixes: &[&str]) -> Vec<String> {
    classes(css)
        .into_iter()
        .filter(|n| {
            !is_ramp_member(n, ramps)
                && !prefixes.iter().any(|r| n.starts_with(r))
                && !mentioned(code, n)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_class_no_source_mentions_is_dead_and_a_longer_name_does_not_save_it() {
        let css = ".a, .b:hover { x: y } /* .c { } */ .w3 { }";
        assert_eq!(dead_classes(css, "class=\"a-b\"", &["w"], &[]), ["a", "b"]);
        assert_eq!(
            dead_classes(css, "a b", &["w"], &[]),
            [] as [std::string::String; 0]
        );
    }
}
