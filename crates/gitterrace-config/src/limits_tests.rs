//! Guards on the `[limits]` knobs: an override reaches the config, and the bounds refuse 0 and
//! absurd values.

use crate::parse;

/// Every bounded limit: id, an in-range override, the value read back, and values the
/// validation must refuse.
type Row = (
    &'static str,
    &'static str,
    fn(&crate::Config) -> u64,
    &'static [&'static str],
);

fn rows() -> Vec<Row> {
    vec![(
        "limits.export_pages",
        "90000",
        |c| c.limits.export_pages as u64,
        &["0", "10000001", "9223372036854775807"],
    )]
}

fn toml_for(id: &str, v: &str) -> String {
    let (sec, name) = id.split_once('.').expect("section.name");
    format!("[{sec}]\n{name} = {v}\n")
}

#[test]
fn every_limit_override_takes_effect() {
    for (id, v, get, _) in rows() {
        let l = parse(&toml_for(id, v)).unwrap_or_else(|e| panic!("{id} = {v}: {e}"));
        assert_eq!(get(&l.config).to_string(), v, "{id}: override ignored");
        assert_ne!(
            get(&crate::Config::default()).to_string(),
            v,
            "{id}: pick a non-default"
        );
    }
}

#[test]
fn every_limit_refuses_zero_and_absurd_values() {
    for (id, _, _, bad) in rows() {
        for v in bad {
            let err = parse(&toml_for(id, v)).err();
            let err = err.unwrap_or_else(|| panic!("{id} = {v} was accepted"));
            assert!(
                err.to_string().contains(id),
                "{id} = {v}: error {err} names no key"
            );
        }
    }
}
