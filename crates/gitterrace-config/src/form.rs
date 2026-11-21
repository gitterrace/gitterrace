//! The fields of the settings form, built from [`keys`] and nothing else: one field per setting,
//! its input from the value's type, its range from [`BOUNDS`], its help from the doc comment.
//! The page renders these values and computes none, so the form cannot drift from the loader.

use crate::schema::{BOUNDS, Config, Key, Reload, keys};
use crate::value::Control;
use crate::{Loaded, Source};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub id: &'static str,
    pub section: &'static str,
    pub name: &'static str,
    pub doc: &'static str,
    pub default: String,
    pub value: String,
    pub source: Source,
    /// Whether a running server takes an edit only after a restart.
    pub restart: bool,
    pub control: Control,
    /// The range of a [`Control::Number`] when [`BOUNDS`] lists the key.
    pub bounds: Option<(u64, u64)>,
}

impl Field {
    /// The value as an input shows it: text without its TOML quotes, a list one entry per line,
    /// anything else as written.
    pub fn plain(&self) -> String {
        let v = toml::from_str::<toml::Table>(&format!("v = {}", self.value))
            .ok()
            .and_then(|mut t| t.remove("v"));
        match v {
            Some(toml::Value::String(s)) => s,
            Some(toml::Value::Array(a)) => {
                a.iter().filter_map(toml::Value::as_str).collect::<Vec<_>>().join("\n")
            }
            Some(_) => self.value.clone(),
            None => String::new(),
        }
    }
}

pub fn fields(loaded: &Loaded) -> Vec<Field> {
    keys().into_iter().map(|k| field(&loaded.config, loaded, k)).collect()
}

fn field(config: &Config, loaded: &Loaded, k: Key) -> Field {
    let value = config.show(&k);
    let bounds = BOUNDS.iter().find(|(id, _, _)| *id == k.id).map(|b| (b.1, b.2));
    Field {
        id: k.id,
        section: k.section,
        name: k.name,
        doc: k.doc,
        value,
        source: loaded.sources.get(k.id).copied().unwrap_or(Source::Default),
        restart: k.reload == Reload::Restart,
        default: k.default,
        bounds: bounds.filter(|_| k.control == Control::Number),
        control: k.control,
    }
}

#[cfg(test)]
mod tests {
    use gitterrace_types::{ColorScheme, DateStyle, LinkStyle, Period, StringEnum};

    use super::fields;
    use crate::value::Control;
    use crate::{BOUNDS, keys, parse};

    fn ids_of(v: &[super::Field]) -> Vec<&'static str> {
        v.iter().map(|f| f.id).collect()
    }

    #[test]
    fn every_key_has_a_control_and_form_ids_equal_schema_ids() {
        let f = fields(&parse("").expect("empty"));
        let schema: Vec<&str> = keys().iter().map(|k| k.id).collect();
        assert_eq!(ids_of(&f), schema);
        let mut seen = std::collections::BTreeSet::new();
        assert!(f.iter().all(|x| seen.insert(x.id)), "a field id repeats");
    }

    #[test]
    fn number_bounds_come_from_bounds() {
        let f = fields(&parse("").expect("empty"));
        for (id, lo, hi) in BOUNDS {
            let x = f.iter().find(|x| x.id == *id).expect("bounded key has a field");
            assert_eq!((&x.control, x.bounds), (&Control::Number, Some((*lo, *hi))), "{id}");
        }
        let free = f.iter().find(|x| x.id == "watch.rescan_secs").expect("key");
        assert_eq!((&free.control, free.bounds), (&Control::Number, None));
    }

    #[test]
    fn select_values_come_from_the_enum() {
        fn all<T: StringEnum>() -> Vec<&'static str> {
            T::ALL.iter().map(|v| v.as_str()).collect()
        }
        let f = fields(&parse("").expect("empty"));
        let sel = |id: &str| f.iter().find(|x| x.id == id).expect(id).control.clone();
        assert_eq!(sel("display.theme"), Control::Select(all::<ColorScheme>()));
        assert_eq!(sel("insights.period"), Control::Select(all::<Period>()));
        assert_eq!(sel("links.style"), Control::Select(all::<LinkStyle>()));
        assert_eq!(sel("display.dates"), Control::Select(all::<DateStyle>()));
        assert_eq!(sel("features.api_enabled"), Control::Tri);
        assert_eq!(sel("server.is_public"), Control::Checkbox);
        assert_eq!(sel("server.hosts"), Control::List);
        assert_eq!(sel("server.addr"), Control::Text);
    }

    #[test]
    fn an_input_shows_text_and_lists_without_toml_quoting() {
        let l = parse("[insights]\nexclude = [\"a/**\", \"b/**\"]\n").expect("ok");
        let f = fields(&l);
        let get = |id: &str| f.iter().find(|x| x.id == id).expect(id).plain();
        assert_eq!(get("insights.exclude"), "a/**\nb/**");
        assert_eq!(get("server.addr"), crate::DEFAULT_ADDR);
        assert_eq!(get("watch.rescan_secs"), "30");
        assert_eq!(get("features.api_enabled"), "");
    }

    #[test]
    fn a_field_shows_the_file_value_and_its_source() {
        let f = fields(&parse("[watch]\nrescan_secs = 50\n").expect("ok"));
        let x = f.iter().find(|x| x.id == "watch.rescan_secs").expect("key");
        assert_eq!((x.value.as_str(), x.source), ("50", crate::Source::File));
        assert_eq!(x.default, "30");
        let addr = f.iter().find(|x| x.id == "server.addr").expect("key");
        assert!(addr.restart);
        assert_eq!(addr.source, crate::Source::Default);
    }
}
