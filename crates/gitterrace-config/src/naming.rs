//! The naming rule for settings and the flags that set them, as one registry.
//!
//! The docs table (`docs/configuration.md`, "Names") and the guard tests both read [`RULES`]
//! and [`EXCEPTIONS`], so the rule as written and the rule as checked cannot drift.

use std::fmt::Write as _;

use crate::keys;

/// One form a name may take.
pub struct Rule {
    /// What the name holds.
    pub holds: &'static str,
    /// The form, as the docs show it.
    pub form: &'static str,
    /// Accepted endings (or prefixes, for a bool).
    pub parts: &'static [&'static str],
    pub example: &'static str,
}

/// A name that breaks its rule on purpose, with why.
pub struct Exception {
    /// A key id, or a flag as `--name`.
    pub name: &'static str,
    /// The key a flag sets, for a terse flag; empty for a key.
    pub sets: &'static str,
    pub why: &'static str,
}

/// Every name form. A size or a time names its unit; a count, the plural it counts.
pub const RULES: &[Rule] = &[
    Rule {
        holds: "a size",
        form: "integer ending in `_kb` or `_mb` (KiB, MiB)",
        parts: &["_kb", "_mb"],
        example: "`limits.text_kb`",
    },
    Rule {
        holds: "a duration",
        form: "integer ending in `_ms` or `_secs`",
        parts: &["_ms", "_secs"],
        example: "`watch.interval_ms`",
    },
    Rule {
        holds: "a count",
        form: "integer ending in the plural it counts",
        parts: &[
            "_lines", "_files", "_commits", "_pages", "_matches", "_streams", "_entries",
            "threads", "_jobs",
        ],
        example: "`api.blame_lines`",
    },
    Rule {
        holds: "a switch",
        form: "bool with a predicate prefix (`is_`, `has_`, `needs_`, `include_`) or the \
               `enabled` gate of its section",
        parts: &["is_", "has_", "needs_", "include_", "enabled"],
        example: "`insights.include_bots`",
    },
    Rule {
        holds: "any key",
        form: "snake_case, never `max` or `min`: the section already says it is a limit",
        parts: &[],
        example: "`limits.log_pages`",
    },
    Rule {
        holds: "a flag that sets a key",
        form: "the key's name with dashes, `--no-<section>` for an `enabled` gate",
        parts: &[],
        example: "`--export-pages` sets `limits.export_pages`",
    },
];

/// The deliberate exceptions. Renamed keys get no alias: an old name is an unknown key, which
/// warns and keeps the default.
pub const EXCEPTIONS: &[Exception] = &[
    Exception {
        name: "multi.depth",
        sets: "",
        why: "a level, not a count",
    },
    Exception {
        name: "index.build_on_start",
        sets: "",
        why: "a predicate prefix hides what it does",
    },
    Exception {
        name: "--public",
        sets: "server.is_public",
        why: "the terse surface of a precise key",
    },
];

fn excepted(name: &str) -> bool {
    EXCEPTIONS.iter().any(|e| e.name == name)
}

/// Every key that breaks [`RULES`], with the rule it breaks.
pub fn key_violations() -> Vec<String> {
    let rule = |holds: &str| {
        RULES
            .iter()
            .find(|r| r.holds == holds)
            .map_or(&[][..], |r| r.parts)
    };
    let mut bad = Vec::new();
    for k in keys() {
        let n = k.name;
        if n.split('_').any(|w| w == "max" || w == "min") {
            bad.push(format!("{}: says max or min", k.id));
        }
        if n != n.to_ascii_lowercase() || n.contains("__") || n.starts_with('_') {
            bad.push(format!("{}: not snake_case", k.id));
        }
        if excepted(k.id) {
            continue;
        }
        let ends = |parts: &[&str]| {
            parts
                .iter()
                .any(|p| n.ends_with(p) || n == p.trim_start_matches('_'))
        };
        match k.ty {
            "integer"
                if !(ends(rule("a size")) || ends(rule("a duration")) || ends(rule("a count"))) =>
            {
                bad.push(format!(
                    "{}: an integer names its unit or the plural it counts",
                    k.id
                ));
            }
            "bool" if !rule("a switch").iter().any(|p| n.starts_with(p) || n == *p) => {
                bad.push(format!("{}: a bool takes a predicate prefix", k.id));
            }
            _ => {}
        }
    }
    bad
}

/// Whether the flag `--long`, documented as overriding `key`, is named by the rule.
pub fn flag_fits(long: &str, key: &str) -> bool {
    if let Some(e) = EXCEPTIONS.iter().find(|e| e.name == format!("--{long}")) {
        return e.sets == key;
    }
    let Some((section, name)) = key.split_once('.') else {
        return false;
    };
    long == name.replace('_', "-") || (name == "enabled" && long == format!("no-{section}"))
}

/// The "Names" section of docs/configuration.md.
pub fn doc_table() -> String {
    let mut s = String::from("| Holds | Form | Example |\n| --- | --- | --- |\n");
    for r in RULES {
        let _ = writeln!(s, "| {} | {} | {} |", r.holds, r.form, r.example);
    }
    s.push_str("\nExceptions, each on purpose:\n\n| Name | Why |\n| --- | --- |\n");
    for e in EXCEPTIONS {
        let sets = if e.sets.is_empty() {
            String::new()
        } else {
            format!(" (sets `{}`)", e.sets)
        };
        let _ = writeln!(s, "| `{}`{sets} | {} |", e.name, e.why);
    }
    s
}

#[cfg(test)]
mod tests {
    use crate::naming::{EXCEPTIONS, flag_fits, key_violations};
    use crate::{keys, parse};

    #[test]
    fn every_key_follows_the_naming_rule() {
        let bad = key_violations();
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    /// An exception names something that exists: a stale one would hide a future break.
    #[test]
    fn every_exception_is_live() {
        let ids: Vec<&str> = keys().iter().map(|k| k.id).collect();
        for e in EXCEPTIONS {
            let key = if e.sets.is_empty() { e.name } else { e.sets };
            assert!(ids.contains(&key), "{}: no key {key}", e.name);
        }
    }

    #[test]
    fn flags_map_to_their_keys() {
        assert!(flag_fits("export-pages", "limits.export_pages"));
        assert!(flag_fits("no-watch", "watch.enabled"));
        assert!(flag_fits("public", "server.is_public"));
        assert!(!flag_fits("max-pages", "limits.export_pages"));
        assert!(!flag_fits("poll", "watch.interval_ms"));
    }

    /// Renamed keys have no alias: each old name is an unknown key, which warns and leaves the
    /// default in force. A shim that still read one would make this fail.
    #[test]
    fn renamed_keys_warn_and_keep_the_default() {
        for (sec, old) in [
            ("server", "max_streams"),
            ("server", "public"),
            ("insights", "bots"),
            ("insights", "generated"),
            ("limits", "log_max_page"),
            ("limits", "fresh_commits"),
            ("limits", "search_per_file"),
            ("api", "log_page_size"),
            ("api", "log_max_skip"),
        ] {
            let v = if ["public", "bots", "generated"].contains(&old) {
                "true"
            } else {
                "3"
            };
            let l = parse(&format!("[{sec}]\n{old} = {v}\n")).expect("parse");
            assert_eq!(
                l.warnings,
                [format!("unknown key `{sec}.{old}` ignored")],
                "{sec}.{old}"
            );
            assert_eq!(
                l.config,
                crate::Config::default(),
                "{sec}.{old} took effect"
            );
        }
    }
}
