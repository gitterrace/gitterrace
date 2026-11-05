//! The settings form's save: change only the keys the form changed, in the text as it is on
//! disk. Comments, key order and spacing stay as written, so a form save and a hand edit of the
//! same keys give the same file.

use std::str::FromStr as _;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};

use crate::locate::{Located, syntax};

/// One change of the form: key `section.name` set to a TOML literal (`true`, `"x"`, `["a"]`),
/// or removed so the default applies (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub value: Option<String>,
}

/// `text` with `changes` applied.
///
/// A key set for the first time goes at the end of its section, the section is added when
/// missing. A row of a `[[list]]` is `list.N.name`; `list.N` with no value takes row `N` out
/// and `N` one past the last row adds one. Refuses text that is not TOML, an id of another shape
/// and a value that is not one TOML literal. The result is not validated here:
/// a save validates whatever text it writes.
pub fn apply(text: &str, changes: &[Change]) -> Result<String, Located> {
    syntax(text)?;
    let mut doc = DocumentMut::from_str(text).map_err(|e| Located {
        line: 0,
        col: 0,
        key: None,
        msg: e.message().to_owned(),
    })?;
    for c in changes {
        let bad = |msg: &str| Located {
            line: 0,
            col: 0,
            key: Some(c.id.clone()),
            msg: format!("{}: {msg}", c.id),
        };
        match c.id.split('.').collect::<Vec<_>>()[..] {
            [array, row] if c.value.is_none() && row.bytes().all(|b| b.is_ascii_digit()) => {
                let row: usize = row.parse().map_err(|_| bad("expected list.N"))?;
                let rows = rows_mut(&mut doc, array).ok_or_else(|| bad("not a list of tables"))?;
                if row >= rows.len() {
                    return Err(bad("no such row"));
                }
                rows.remove(row);
            }
            [section, name] if !section.is_empty() && !name.is_empty() => match &c.value {
                Some(lit) => {
                    let v = Value::from_str(lit).map_err(|_| bad("expected one TOML value"))?;
                    let t = section_mut(&mut doc, section).ok_or_else(|| bad("not a table"))?;
                    set(t, name, v);
                }
                None => {
                    if let Some(t) = doc.get_mut(section).and_then(Item::as_table_mut) {
                        t.remove(name);
                    }
                }
            },
            [array, row, name] => {
                let row: usize = row.parse().map_err(|_| bad("expected list.N.name"))?;
                let rows = rows_mut(&mut doc, array).ok_or_else(|| bad("not a list of tables"))?;
                if row == rows.len() {
                    rows.push(Table::new());
                }
                let t = rows.get_mut(row).ok_or_else(|| bad("no such row"))?;
                match &c.value {
                    Some(literal) => {
                        let v =
                            Value::from_str(literal).map_err(|_| bad("expected one TOML value"))?;
                        set(t, name, v);
                    }
                    None => {
                        t.remove(name);
                    }
                }
            }
            _ => return Err(bad("expected section.name or list.N.name")),
        }
    }
    Ok(doc.to_string())
}

/// The `[[list]]` rows, added at the end when the text has none.
fn rows_mut<'a>(doc: &'a mut DocumentMut, list: &str) -> Option<&'a mut ArrayOfTables> {
    let item = doc
        .entry(list)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    item.as_array_of_tables_mut()
}

/// The `[section]` table, added at the end when the text has none.
fn section_mut<'a>(doc: &'a mut DocumentMut, section: &str) -> Option<&'a mut Table> {
    let item = doc.entry(section).or_insert_with(toml_edit::table);
    item.as_table_mut()
}

/// Sets `name`. An existing value is replaced in place: the comment above sits on the key and
/// the one after on the value, so both stay.
fn set(table: &mut Table, name: &str, mut v: Value) {
    match table.get_mut(name).and_then(Item::as_value_mut) {
        Some(old) => {
            *v.decor_mut() = old.decor().clone();
            *old = v;
        }
        None => {
            table.insert(name, Item::Value(v));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::edit::{Change, apply};

    fn set(id: &str, v: &str) -> Change {
        Change {
            id: id.into(),
            value: Some(v.into()),
        }
    }

    const FILE: &str = "# my settings\n\n[watch]\n# how often\ninterval_ms = 500 # fast\n\
                        enabled = true\n\n[display]\ntheme = \"dark\"\n";

    #[test]
    fn a_save_keeps_comments_and_key_order() {
        let out = apply(FILE, &[set("watch.interval_ms", "2000")]).expect("apply");
        assert_eq!(out, FILE.replace("500", "2000"));
        let out = apply(
            FILE,
            &[set("watch.rescan_secs", "60"), set("links.style", "\"go\"")],
        )
        .expect("apply");
        assert!(
            out.starts_with(FILE.split("\n[display]").next().expect("head")),
            "{out}"
        );
        assert!(out.contains("enabled = true\nrescan_secs = 60\n"), "{out}");
        assert!(out.ends_with("[links]\nstyle = \"go\"\n"), "{out}");
    }

    #[test]
    fn a_removed_key_leaves_the_rest() {
        let out = apply(
            FILE,
            &[Change {
                id: "watch.enabled".into(),
                value: None,
            }],
        )
        .expect("apply");
        assert_eq!(out, FILE.replace("enabled = true\n", ""));
    }

    const ACTIONS: &str = "version = 1\n\n# mine\n[[action]]\nid = \"a\"\nlabel = \"A\" # first\n\n\
                           [[action]]\nid = \"b\"\nlabel = \"B\"\n";

    #[test]
    fn list_rows_change_add_and_go_keeping_comments() {
        let out = apply(ACTIONS, &[set("action.0.label", "\"Ay\"")]).expect("apply");
        assert_eq!(out, ACTIONS.replace("\"A\" #", "\"Ay\" #"));
        let out = apply(
            ACTIONS,
            &[set("action.2.id", "\"c\""), set("action.2.label", "\"C\"")],
        )
        .expect("add");
        assert!(out.starts_with(ACTIONS), "{out}");
        assert!(
            out.ends_with("[[action]]\nid = \"c\"\nlabel = \"C\"\n"),
            "{out}"
        );
        let gone = Change {
            id: "action.1".into(),
            value: None,
        };
        let out = apply(ACTIONS, &[gone]).expect("remove");
        assert!(out.contains("# mine") && !out.contains("\"b\""), "{out}");
        for c in [set("action.5.id", "\"x\""), set("version.0.id", "\"x\"")] {
            assert!(
                apply(ACTIONS, std::slice::from_ref(&c)).is_err(),
                "{}",
                c.id
            );
        }
    }

    #[test]
    fn bad_input_is_refused_with_its_place() {
        let e = apply("[watch\n", &[]).expect_err("syntax");
        assert_eq!(e.line, 1);
        for c in [
            set("watch", "1"),
            set("a.b.c", "1"),
            set("watch.enabled", "1 2"),
        ] {
            let e = apply(FILE, std::slice::from_ref(&c)).expect_err(&c.id);
            assert_eq!(e.key.as_deref(), Some(c.id.as_str()));
        }
    }
}
