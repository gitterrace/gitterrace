//! Guards on the bounded limits: an override reaches the config, the bounds refuse 0 and
//! absurd values, and the public-cost tag covers exactly the per-request knobs.

use crate::{BOUNDS, Config, keys, parse};

fn toml_for(id: &str, v: u64) -> String {
    let (sec, name) = id.split_once('.').expect("section.name");
    format!("[{sec}]\n{name} = {v}\n")
}

fn value(c: &Config, id: &str) -> u64 {
    let k = keys().into_iter().find(|k| k.id == id).expect("known key");
    c.show(&k).parse().expect("a number")
}

/// Every bounded key, at its least and its most, reads back as set.
#[test]
fn every_limit_override_takes_effect() {
    for (id, lo, hi) in BOUNDS {
        for v in [*lo, *hi] {
            let l = parse(&toml_for(id, v)).unwrap_or_else(|e| panic!("{id} = {v}: {e}"));
            assert_eq!(value(&l.config, id), v, "{id}: override ignored");
        }
    }
}

#[test]
fn every_limit_refuses_zero_and_absurd_values() {
    for (id, _, hi) in BOUNDS {
        for v in [0, hi + 1, i64::MAX.unsigned_abs()] {
            let err = parse(&toml_for(id, v)).err();
            let err = err.unwrap_or_else(|| panic!("{id} = {v} was accepted"));
            assert!(
                err.to_string().contains(id),
                "{id} = {v}: error {err} names no key"
            );
        }
    }
}

/// Every default sits inside its bounds, and every integer under [limits] or [api] is bounded:
/// a new limit without a range fails here.
#[test]
fn every_limit_is_bounded_and_its_default_fits() {
    let d = Config::default();
    for (id, lo, hi) in BOUNDS {
        let k = keys().into_iter().find(|k| k.id == *id).expect("known key");
        if d.show(&k) == "unset" {
            continue;
        }
        let v = value(&d, id);
        assert!(
            (*lo..=*hi).contains(&v),
            "{id}: default {v} outside {lo}..={hi}"
        );
    }
    for k in keys() {
        if ["limits", "api"].contains(&k.section)
            && ![
                "limits.text_kb",
                "limits.raw_mb",
                "limits.search_matches",
                "limits.diff_fold_lines",
            ]
            .contains(&k.id)
        {
            assert!(BOUNDS.iter().any(|b| b.0 == k.id), "{}: no bounds", k.id);
        }
    }
}

/// The keys a public server warns about. A knob that makes one request cost more must be tagged
/// `cost` in the schema, so it shows up here; this list changes only with the schema.
#[test]
fn cost_tagged_keys_are_the_per_request_knobs() {
    let tagged: Vec<&str> = keys().iter().filter(|k| k.cost).map(|k| k.id).collect();
    assert_eq!(
        tagged,
        [
            "server.public_timeout_secs",
            "limits.text_kb",
            "limits.raw_mb",
            "limits.search_matches",
            "limits.search_file_lines",
            "limits.search_commits",
            "limits.diff_fold_lines",
            "limits.blob_lines",
            "limits.highlight_kb",
            "limits.log_pages",
            "limits.preview_mb",
            "limits.unindexed_commits",
            "api.log_page_commits",
            "api.log_skip_commits",
            "api.blob_lines",
            "api.blob_kb",
            "api.blame_lines",
            "api.diff_lines",
        ]
    );
    // Every numeric key under [api] costs per request, so none may be left untagged.
    for k in keys().iter().filter(|k| k.section == "api") {
        assert!(k.cost, "{}: an [api] limit without the cost tag", k.id);
    }
}

#[test]
fn costs_above_default_name_only_raised_keys() {
    let mut c = Config::default();
    assert!(c.costs_above_default().is_empty());
    c.api.diff_lines += 1;
    c.limits.search_matches += 1;
    c.limits.blob_lines -= 1;
    c.limits.registry_entries += 1;
    assert_eq!(
        c.costs_above_default(),
        ["limits.search_matches", "api.diff_lines"]
    );
}
