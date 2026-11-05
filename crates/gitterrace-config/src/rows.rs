//! The rows of a file of `[[list]]` tables, for the actions form: each row's values as its
//! inputs hold them, and the TOML literal an input means. The keys and their types come from
//! the caller (the actions schema), so this crate does not know the actions file.

use std::str::FromStr as _;

use toml_edit::{Document, Value};

/// One key of a row, as the schema lists it: its name and its documented type.
#[derive(Debug, Clone, Copy)]
pub struct RowKey {
    pub name: &'static str,
    /// `string`, `boolean`, `array of strings`; any other type is edited in Plain.
    pub ty: &'static str,
}

/// The rows of `[[list]]` in `text`: per row, each key the row sets, as its input shows it.
/// `None` when `text` is not TOML.
pub fn rows(text: &str, list: &str, keys: &[RowKey]) -> Option<Vec<Vec<(&'static str, String)>>> {
    let doc = Document::parse(text).ok()?;
    let tables = doc.get(list).and_then(toml_edit::Item::as_array_of_tables);
    Some(tables.map_or_else(Vec::new, |a| {
        a.iter()
            .map(|t| {
                keys.iter()
                    .filter_map(|k| {
                        let v = t.get(k.name)?.as_value()?;
                        Some((k.name, input_of(v)))
                    })
                    .collect()
            })
            .collect()
    }))
}

/// A value as its input holds it: a string unquoted, a list one item per line, else its TOML.
fn input_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.value().clone(),
        Value::Array(a) if a.iter().all(|x| x.as_str().is_some()) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        other => other.to_string().trim().to_owned(),
    }
}

/// Whether the form edits a key of type `ty`; the rest are shown and edited in Plain.
pub fn row_editable(ty: &str) -> bool {
    matches!(ty, "string" | "boolean" | "array of strings")
}

/// The TOML literal input `input` means for a key of type `ty`. `None` for a type the form
/// does not edit or an input that is no value of the type.
pub fn row_literal(ty: &str, input: &str) -> Option<String> {
    match ty {
        "boolean" => matches!(input, "true" | "false").then(|| input.to_owned()),
        "array of strings" => {
            let items = input.lines().map(str::trim).filter(|l| !l.is_empty());
            Some(items.collect::<toml_edit::Array>().to_string())
        }
        "string" => Some(Value::from(input).to_string()),
        _ => None,
    }
    .filter(|l| Value::from_str(l).is_ok())
}

#[cfg(test)]
mod tests {
    use super::{RowKey, row_literal, rows};

    const KEYS: [RowKey; 3] = [
        RowKey {
            name: "id",
            ty: "string",
        },
        RowKey {
            name: "kinds",
            ty: "array of strings",
        },
        RowKey {
            name: "command",
            ty: "string or array of strings",
        },
    ];

    #[test]
    fn rows_show_strings_unquoted_and_lists_one_per_line() {
        let text = "version = 1\n[[action]]\nid = \"e\"\nkinds = [\"file\", \"line\"]\n\
                    command = [\"code\", \"-g\"]\n";
        let r = rows(text, "action", &KEYS).expect("toml");
        assert_eq!(
            r,
            [vec![
                ("id", "e".to_owned()),
                ("kinds", "file\nline".to_owned()),
                ("command", "code\n-g".to_owned()),
            ]]
        );
        assert_eq!(
            rows(text, "send", &KEYS).expect("toml"),
            Vec::<Vec<_>>::new()
        );
        assert!(rows("[[action]\n", "action", &KEYS).is_none());
    }

    #[test]
    fn inputs_become_literals_by_type() {
        assert_eq!(row_literal("string", "M\"x").as_deref(), Some("'M\"x'"));
        assert_eq!(
            row_literal("array of strings", "file\n\nline").as_deref(),
            Some("[\"file\", \"line\"]")
        );
        assert_eq!(row_literal("boolean", "false").as_deref(), Some("false"));
        assert_eq!(row_literal("boolean", "yes"), None);
        assert_eq!(row_literal("table", "x"), None);
    }
}
