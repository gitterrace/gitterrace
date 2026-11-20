//! The rows of a file of `[[list]]` tables, for the actions form: each row's values as its
//! inputs hold them, and the TOML literal an input means. The keys and their types come from
//! the caller (the actions schema), so this crate does not know the actions file.

use std::collections::BTreeMap;
use std::str::FromStr as _;

use toml_edit::{Document, Value};

/// One key of a row, as the schema lists it: its name and its documented type.
#[derive(Debug, Clone, Copy)]
pub struct RowKey {
    pub name: &'static str,
    /// `string`, `boolean`, `array of strings`, `table of strings`; any other type is edited in
    /// Plain.
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
                        let item = t.get(k.name)?;
                        let input = match item.as_table_like() {
                            Some(tab) => table_input(tab),
                            None => input_of(item.as_value()?),
                        };
                        Some((k.name, input))
                    })
                    .collect()
            })
            .collect()
    }))
}

/// A table of strings as its input holds it: one `name = value` per line, whether the file
/// has it inline or as a sub-table. A non-string value shows as its TOML.
fn table_input(t: &dyn toml_edit::TableLike) -> String {
    let line = |(k, v): (&str, &toml_edit::Item)| {
        let shown = v.as_value().map_or_else(String::new, |v| match v {
            Value::String(s) => s.value().clone(),
            other => other.to_string().trim().to_owned(),
        });
        format!("{k} = {shown}")
    };
    t.iter().map(line).collect::<Vec<_>>().join("\n")
}

/// A value as its input holds it: a string unquoted, a list one item per line, else its TOML.
fn input_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.value().clone(),
        Value::Array(a) if a.iter().all(|x| x.as_str().is_some()) => {
            a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")
        }
        other => other.to_string().trim().to_owned(),
    }
}

/// Whether the form edits a key of type `ty`; the rest are shown and edited in Plain.
pub fn row_editable(ty: &str) -> bool {
    matches!(ty, "string" | "boolean" | "array of strings" | "table of strings")
}

/// The inline table `input` means, one `name = value` per line.
///
/// `check` is the loader's own rule for the entries (names, count, sizes): its reason is the
/// error. A repeated name keeps the last value, as two form rows of one name would mean.
pub fn table_literal(
    input: &str,
    check: impl Fn(&BTreeMap<String, String>) -> Result<(), String>,
) -> Result<String, String> {
    let mut map = BTreeMap::new();
    for line in input.lines().filter(|l| !l.trim().is_empty()) {
        let (name, value) =
            line.split_once('=').ok_or_else(|| format!("{line:?}: write name = value"))?;
        map.insert(name.trim().to_owned(), value.trim().to_owned());
    }
    check(&map)?;
    let mut t = toml_edit::InlineTable::new();
    for (k, v) in &map {
        t.insert(k, Value::from(v.as_str()));
    }
    Ok(t.to_string())
}

/// The TOML literal input `input` means for a key of type `ty`.
///
/// `None` for a type the form does not edit or an input that is no value of the type. A table of
/// strings needs the loader's rules: see [`table_literal`].
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
    use super::{RowKey, row_editable, row_literal, rows, table_literal};

    const KEYS: [RowKey; 3] = [
        RowKey { name: "id", ty: "string" },
        RowKey { name: "kinds", ty: "array of strings" },
        RowKey { name: "command", ty: "string or array of strings" },
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
        assert_eq!(rows(text, "send", &KEYS).expect("toml"), Vec::<Vec<_>>::new());
        assert!(rows("[[action]\n", "action", &KEYS).is_none());
    }

    const VARS: [RowKey; 1] = [RowKey { name: "vars", ty: "table of strings" }];

    #[test]
    fn rows_show_vars_from_inline_and_sub_table_forms() {
        let inline = "[[action]]\nvars = { a = \"x y\", b = \"q\\\\\" }\n";
        let sub = "[[action]]\n[action.vars]\na = \"x y\"\nb = \"q\\\\\"\n";
        for t in [inline, sub] {
            let r = rows(t, "action", &VARS).expect("toml");
            assert_eq!(r, [vec![("vars", "a = x y\nb = q\\".to_owned())]], "{t}");
        }
        let none = rows("[[action]]\nid = \"a\"\n", "action", &VARS).expect("toml");
        assert_eq!(none, [vec![]]);
        let empty = rows("[[action]]\nvars = {}\n", "action", &VARS).expect("toml");
        assert_eq!(empty, [vec![("vars", String::new())]]);
    }

    fn few(m: &std::collections::BTreeMap<String, String>) -> Result<(), String> {
        if m.len() > 2 { Err("too many".into()) } else { Ok(()) }
    }

    #[test]
    fn table_literal_quotes_values_and_applies_the_check() {
        assert_eq!(table_literal("", few).as_deref(), Ok("{}"));
        assert_eq!(table_literal("a = 1\n\nb=2", few).as_deref(), Ok("{ a = \"1\", b = \"2\" }"));
        let tricky = table_literal("a = say \"hi\" \\ ok", few).expect("lit");
        let parsed: toml_edit::Value = tricky.parse().expect("toml");
        let got = parsed.as_inline_table().and_then(|t| t.get("a")).and_then(|v| v.as_str());
        assert_eq!(got, Some("say \"hi\" \\ ok"));
        assert_eq!(table_literal("a = 1\nb = 2\nc = 3", few), Err("too many".into()));
        assert!(table_literal("no equals", few).is_err());
    }

    #[test]
    fn vars_are_editable_but_row_literal_leaves_them_to_table_literal() {
        assert!(row_editable("table of strings"));
        assert_eq!(row_literal("table of strings", "a = 1"), None);
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
