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
    /// `checkbox`, `list`, `kv` (one `name = value` per line), `text`, or `plain` for a key the
    /// form leaves to Plain.
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
                "table of strings" => "kv",
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
/// `Err` names why: a key the schema does not list or the form does not edit, or a table the
/// loader would refuse (`vars` go through the loader's own check).
pub(crate) fn literal(id: &str, input: &str) -> Result<String, String> {
    let not_taken = || format!("{id}: not a value this key takes");
    let mut parts = id.split('.');
    let (Some(list), Some(_), Some(name)) = (parts.next(), parts.next(), parts.next()) else {
        return Err(not_taken());
    };
    let key = LISTS
        .iter()
        .find(|(l, _)| *l == list)
        .and_then(|(_, keys)| keys.iter().find(|k| k.name == name));
    let Some(k) = key else {
        return Err(not_taken());
    };
    if k.ty == "table of strings" {
        return gitterrace_config::table_literal(input, gitterrace_actions::config::check_vars)
            .map_err(|why| format!("{id}: {why}"));
    }
    gitterrace_config::row_literal(k.ty, input).ok_or_else(not_taken)
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
        assert_eq!(literal("send.0.label", "M\"x").as_deref(), Ok("'M\"x'"));
        assert_eq!(
            literal("action.1.kinds", "file\n\nline").as_deref(),
            Ok("[\"file\", \"line\"]")
        );
        for id in ["action.0.command", "send.0.nope", "repo.0.id"] {
            assert!(literal(id, "x").is_err(), "{id}");
        }
    }

    #[test]
    fn vars_input_uses_the_loader_rules_and_the_kv_kind() {
        let l = lists(FILE).expect("toml");
        let vars = l[1]
            .blank
            .fields
            .iter()
            .find(|f| f.name == "vars")
            .expect("vars");
        assert_eq!(vars.kind, "kv");
        assert_eq!(
            literal("action.0.vars", "a = 1\nb_2 = x").as_deref(),
            Ok("{ a = \"1\", b_2 = \"x\" }")
        );
        let bad = literal("action.0.vars", "Bad-Name = x").expect_err("name");
        assert!(
            bad.contains("action.0.vars") && bad.contains("vars."),
            "{bad}"
        );
        let many = (0..33).fold(String::new(), |acc, i| acc + &format!("v{i} = x\n"));
        assert!(
            literal("action.0.vars", &many)
                .expect_err("many")
                .contains("at most 32")
        );
    }

    #[test]
    fn vars_roundtrip_keeps_comments_and_other_keys() {
        let text = "version = 1\n# mine\n[[action]]\nid = \"e\" # id\nlabel = \"E\"\n\
                    command = [\"x\"]\n\n[action.vars]\nold = \"1\"\n";
        let l = lists(text).expect("toml");
        let v = l[1].rows[0]
            .fields
            .iter()
            .find(|f| f.name == "vars")
            .expect("vars");
        assert_eq!(v.value, "old = 1");
        let c = gitterrace_config::Change {
            id: "action.0.vars".into(),
            value: literal("action.0.vars", "old = 1\nnew = q\"\\z").ok(),
        };
        let out = gitterrace_config::apply(text, &[c]).expect("apply");
        assert!(
            out.contains("# mine") && out.contains("id = \"e\" # id"),
            "{out}"
        );
        assert!(out.contains("command = [\"x\"]"), "{out}");
        let back = lists(&out).expect("toml");
        let v = back[1].rows[0]
            .fields
            .iter()
            .find(|f| f.name == "vars")
            .expect("vars");
        assert_eq!(v.value, "new = q\"\\z\nold = 1");
    }

    #[test]
    fn roundtrip_keeps_comments() {
        let c = gitterrace_config::Change {
            id: "send.0.label".into(),
            value: literal("send.0.label", "Mail draft").ok(),
        };
        let out = gitterrace_config::apply(FILE, &[c]).expect("apply");
        assert_eq!(out, FILE.replace("\"Mail\"", "\"Mail draft\""));
    }
}
