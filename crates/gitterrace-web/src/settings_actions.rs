//! The `actions.toml` form: one card per `[[send]]` and `[[action]]` row. Its fields come from
//! `gitterrace_actions::schema` (`SEND_KEYS`, `ACTION_KEYS`), the tables the docs are rendered
//! from, so the form lists exactly the keys the loader reads. A key whose type has no simple
//! input (a table, a command that may be a list or a string) is shown and edited in Plain.
//! Edits go through the same check and save routes as `config.toml`, with ids `list.N.name`.

use gitterrace_actions::schema::{ACTION_KEYS, Item, SEND_KEYS};
use gitterrace_config::RowKey;

/// The lists of tables the form edits, with the keys of one row.
pub(crate) const LISTS: [(&str, &[Item]); 2] = [("send", &SEND_KEYS), ("action", &ACTION_KEYS)];

/// One list (`[[send]]`) as the form shows it.
pub(crate) struct List {
    pub(crate) name: &'static str,
    pub(crate) rows: Vec<Card>,
    /// The inputs of a new row, numbered one past the last.
    pub(crate) blank: Card,
}

/// One row of a list.
pub(crate) struct Card {
    /// `send.2`.
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) fields: Vec<CardField>,
}

/// One key of a row.
pub(crate) struct CardField {
    /// `send.2.label`, the id an edit names.
    pub(crate) id: String,
    pub(crate) name: &'static str,
    pub(crate) doc: &'static str,
    pub(crate) required: bool,
    /// `checkbox`, `list`, `text`, or `plain` for a key the form leaves to Plain.
    pub(crate) kind: &'static str,
    pub(crate) value: String,
    pub(crate) checked: bool,
}

fn row_keys(keys: &[Item]) -> Vec<RowKey> {
    keys.iter()
        .map(|k| RowKey {
            name: k.name,
            ty: k.ty,
        })
        .collect()
}

/// The lists of `text`, `None` when it is not TOML.
pub(crate) fn lists(text: &str) -> Option<Vec<List>> {
    let mut out = Vec::new();
    for (name, keys) in LISTS {
        let rows = gitterrace_config::rows(text, name, &row_keys(keys))?;
        let cards: Vec<Card> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| card(name, i, keys, r))
            .collect();
        let blank = card(name, cards.len(), keys, &[]);
        out.push(List {
            name,
            rows: cards,
            blank,
        });
    }
    Some(out)
}

fn card(list: &str, i: usize, keys: &[Item], set: &[(&'static str, String)]) -> Card {
    let get = |k: &str| set.iter().find(|(n, _)| *n == k).map(|(_, v)| v.as_str());
    let title = get("label")
        .or_else(|| get("id"))
        .unwrap_or("New")
        .to_owned();
    let fields = keys
        .iter()
        .map(|k| {
            let value = get(k.name).unwrap_or_default().to_owned();
            let kind = match k.ty {
                ty if !gitterrace_config::row_editable(ty) => "plain",
                "boolean" => "checkbox",
                "array of strings" => "list",
                _ => "text",
            };
            CardField {
                id: format!("{list}.{i}.{}", k.name),
                name: k.name,
                doc: k.doc,
                required: k.required,
                kind,
                checked: get(k.name).map_or(k.default == "`true`", |v| v == "true"),
                value,
            }
        })
        .collect();
    Card {
        id: format!("{list}.{i}"),
        title,
        fields,
    }
}

/// The TOML literal a form input means for `list.N.name`, by the key's type in the schema.
/// `None` for a key the schema does not list or the form does not edit.
pub(crate) fn literal(id: &str, input: &str) -> Option<String> {
    let mut parts = id.split('.');
    let (list, _, name) = (parts.next()?, parts.next()?, parts.next()?);
    let keys = LISTS.iter().find(|(l, _)| *l == list)?.1;
    let k = keys.iter().find(|k| k.name == name)?;
    gitterrace_config::row_literal(k.ty, input)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gitterrace_actions::schema::{ACTION_KEYS, SEND_KEYS};

    use crate::settings_actions::{lists, literal};

    const FILE: &str = "version = 1\n# mine\n[[send]]\nid = \"mail\"\nlabel = \"Mail\"\n\
                        url = \"mailto:?subject={title}\"\n\n[[action]]\nid = \"e\"\n\
                        label = \"Edit\"\nkinds = [\"file\", \"line\"]\ncommand = [\"code\"]\n";

    #[test]
    fn action_form_fields_equal_the_actions_schema() {
        let l = lists(FILE).expect("toml");
        for (list, keys) in [("send", &SEND_KEYS[..]), ("action", &ACTION_KEYS[..])] {
            let want: BTreeSet<&str> = keys.iter().map(|k| k.name).collect();
            let got = l.iter().find(|x| x.name == list).expect(list);
            for card in got.rows.iter().chain(std::iter::once(&got.blank)) {
                let names: BTreeSet<&str> = card.fields.iter().map(|f| f.name).collect();
                assert_eq!(names, want, "{list}: the form and the schema disagree");
            }
        }
    }

    #[test]
    fn rows_show_their_values_and_number_from_zero() {
        let l = lists(FILE).expect("toml");
        let action = &l[1].rows[0];
        assert_eq!(
            (action.id.as_str(), action.title.as_str()),
            ("action.0", "Edit")
        );
        let kinds = action
            .fields
            .iter()
            .find(|f| f.name == "kinds")
            .expect("kinds");
        assert_eq!((kinds.kind, kinds.value.as_str()), ("list", "file\nline"));
        assert_eq!(l[0].blank.id, "send.1");
        assert!(lists("[[send]\n").is_none());
    }

    #[test]
    fn inputs_become_literals_by_the_schema_type() {
        assert_eq!(literal("send.0.label", "M\"x").as_deref(), Some("'M\"x'"));
        assert_eq!(
            literal("action.1.kinds", "file\n\nline").as_deref(),
            Some("[\"file\", \"line\"]")
        );
        for id in ["action.0.command", "send.0.nope", "repo.0.id"] {
            assert_eq!(literal(id, "x"), None, "{id}");
        }
    }

    #[test]
    fn roundtrip_keeps_comments() {
        let c = gitterrace_config::Change {
            id: "send.0.label".into(),
            value: literal("send.0.label", "Mail draft"),
        };
        let out = gitterrace_config::apply(FILE, &[c]).expect("apply");
        assert_eq!(out, FILE.replace("\"Mail\"", "\"Mail draft\""));
    }
}
