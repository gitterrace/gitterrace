//! Send to: `[[send]]` targets are URL templates the browser opens. No process, no POST and
//! no token, so gitterrace stays read-only. The server never builds the final URL: it hands
//! the page the template as parts, and the page fills each placeholder with [`fill`]'s rule
//! (`static/send.js` mirrors it, `testdata/send-vectors.json` holds both to it).
//!
//! Every check runs at load, on the template after `{var.NAME}` is substituted: `vars` come
//! from the user's own config, are inserted verbatim, and may therefore form the base URL.
//! Page placeholders are always percent-encoded and may never reach the scheme or the host.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{Piece, pieces, url_encode};

/// Placeholders the page fills, in the order the docs list them.
pub const SEND_FIELDS: [&str; 7] = [
    "title",
    "notes",
    "permalink",
    "path",
    "lines",
    "sha",
    "repo",
];

/// Schemes refused even though they look like an app scheme: each runs code, reads local
/// data or reaches browser internals.
pub const DENIED_SCHEMES: [&str; 8] = [
    "javascript",
    "data",
    "file",
    "vbscript",
    "blob",
    "about",
    "chrome",
    "view-source",
];

/// A built-in target: `preset = "<name>"` takes this template; `vars` lists the settings it
/// needs in `[send.vars]`.
pub struct Preset {
    pub name: &'static str,
    /// The label when the block sets none.
    pub label: &'static str,
    pub url: &'static str,
    pub vars: &'static [&'static str],
    pub doc: &'static str,
}

pub const PRESETS: [Preset; 4] = [
    Preset {
        name: "tendir",
        label: "Send to tendir",
        url: "tendir://new-task?title={title}&notes={notes}",
        vars: &[],
        doc: "tendir's new-task sheet, filled in.",
    },
    Preset {
        name: "github-issue",
        label: "New issue",
        url: "https://github.com/{var.repo}/issues/new?title={title}&body={notes}",
        vars: &["repo"],
        doc: "A new issue on github.com. `repo` is `owner/name`.",
    },
    Preset {
        name: "forgejo-issue",
        label: "New issue",
        url: "{var.base}/{var.repo}/issues/new?title={title}&body={notes}",
        vars: &["base", "repo"],
        doc: "A new issue on a self-hosted forge. `base` is the instance URL, `repo` is `owner/name`.",
    },
    Preset {
        name: "mail",
        label: "Mail draft",
        url: "mailto:?subject={title}&body={notes}",
        vars: &[],
        doc: "A mail draft in your mail app.",
    },
];

/// One part of a template as the page receives it: `{"lit": "..."}` is copied as is,
/// `{"ph": "title"}` is replaced by the encoded value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    Lit(String),
    Ph(String),
}

/// One validated target, the shape `GET /_send/targets` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Target {
    pub id: String,
    pub label: String,
    pub parts: Vec<Part>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawSend {
    id: String,
    label: Option<String>,
    url: Option<String>,
    preset: Option<String>,
    #[serde(default)]
    vars: BTreeMap<String, String>,
}

/// Fills `parts` with `values`: each value percent-encoded with the `:url` rule, a missing
/// value as the empty string.
pub fn fill(parts: &[Part], values: &BTreeMap<String, String>) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Lit(s) => s.clone(),
            Part::Ph(n) => url_encode(values.get(n).map_or("", String::as_str)),
        })
        .collect()
}

impl RawSend {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

/// Validates one `[[send]]` block. The error is the reason, without the id.
pub(crate) fn compile(raw: RawSend) -> Result<Target, String> {
    for (name, value) in &raw.vars {
        if value.len() > 4096 || value.contains(['\0', '\n', '\r']) {
            return Err(format!("vars.{name}: one line, at most 4 KiB"));
        }
    }
    let (template, label) = match (&raw.url, &raw.preset) {
        (Some(_), Some(_)) => return Err("set exactly one of url and preset".into()),
        (None, None) => return Err("set url or preset".into()),
        (Some(u), None) => (u.clone(), raw.label.clone()),
        (None, Some(name)) => {
            let p = PRESETS.iter().find(|p| p.name == name).ok_or_else(|| {
                let known: Vec<_> = PRESETS.iter().map(|p| p.name).collect();
                format!("unknown preset {name:?}; known: {}", known.join(", "))
            })?;
            for v in p.vars {
                if !raw.vars.contains_key(*v) {
                    return Err(format!("preset {name} needs vars.{v}"));
                }
            }
            (
                p.url.to_owned(),
                raw.label.clone().or_else(|| Some(p.label.to_owned())),
            )
        }
    };
    let label = label.ok_or("label is required with url")?;
    if label.trim().is_empty() || label.chars().count() > 80 {
        return Err("label must be 1 to 80 characters".into());
    }
    let mut parts: Vec<Part> = Vec::new();
    let push_lit = |parts: &mut Vec<Part>, s: &str| match parts.last_mut() {
        Some(Part::Lit(l)) => l.push_str(s),
        _ => parts.push(Part::Lit(s.to_owned())),
    };
    for piece in pieces(&template)? {
        match piece {
            Piece::Text(t) => push_lit(&mut parts, &t),
            Piece::Field(f) => {
                if let Some(v) = f.strip_prefix("var.") {
                    let value = raw
                        .vars
                        .get(v)
                        .ok_or_else(|| format!("url: {{{f}}}: no vars.{v}"))?;
                    push_lit(&mut parts, value);
                } else if SEND_FIELDS.contains(&f.as_str()) {
                    parts.push(Part::Ph(f));
                } else {
                    return Err(format!(
                        "url: unknown placeholder {{{f}}}; known: {}, var.<name>",
                        SEND_FIELDS.join(", ")
                    ));
                }
            }
        }
    }
    let prefix = match parts.first() {
        Some(Part::Lit(l)) => l.as_str(),
        _ => "",
    };
    check_prefix(prefix, parts.len() > 1)?;
    Ok(Target {
        id: raw.id,
        label,
        parts,
    })
}

/// The literal text before the first page placeholder must fix the scheme and, when a
/// placeholder follows, end the host: a value can then only land in the path, query or
/// fragment.
fn check_prefix(prefix: &str, has_placeholder: bool) -> Result<(), String> {
    let (scheme, rest) = prefix
        .split_once(':')
        .ok_or("url: the scheme must be literal, before any placeholder")?;
    let lower = scheme.to_ascii_lowercase();
    let well_formed = lower.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && lower
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b));
    if !well_formed {
        return Err(format!("url: bad scheme {scheme:?}"));
    }
    if DENIED_SCHEMES.contains(&lower.as_str()) {
        return Err(format!("url: scheme {lower}: is refused"));
    }
    let after = rest.strip_prefix("//").unwrap_or(rest);
    let host_end = after.find(['/', '?', '#']);
    if has_placeholder && host_end.is_none() {
        return Err("url: a placeholder may not be part of the scheme or host".into());
    }
    if lower == "http" {
        let authority = &after[..host_end.unwrap_or(after.len())];
        let host = authority.rsplit('@').next().unwrap_or_default();
        let host = match host.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or_default(),
            None => host.split(':').next().unwrap_or_default(),
        };
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !loopback {
            return Err("url: http is allowed only for loopback; use https".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use serde::Deserialize;

    use super::*;
    use crate::config::Config;

    fn load(body: &str) -> Result<Config, String> {
        Config::parse(
            &format!("version = 1\n[[send]]\nid = \"s\"\n{body}"),
            Path::new("/"),
        )
        .map_err(|e| e.to_string())
    }

    fn url(u: &str) -> Result<Config, String> {
        load(&format!("label = \"S\"\nurl = {u:?}\n"))
    }

    #[test]
    fn scheme_table() {
        // Written out, not DENIED_SCHEMES: dropping an entry there must fail here.
        for s in [
            "javascript",
            "data",
            "file",
            "vbscript",
            "blob",
            "about",
            "chrome",
            "view-source",
        ] {
            let e = url(&format!("{s}:x?t={{title}}")).expect_err(s);
            assert!(e.contains("refused"), "{s}: {e}");
            let upper = s.to_ascii_uppercase();
            assert!(url(&format!("{upper}:x?t={{title}}")).is_err(), "{upper}");
        }
        for ok in [
            "https://example.test/new?t={title}",
            "http://localhost:8080/new?t={title}",
            "http://127.0.0.1/new?t={title}",
            "http://[::1]/new?t={title}",
            "mailto:?subject={title}",
            "things:///add?title={title}",
            "x-app+v1.2://new?t={title}",
        ] {
            assert!(url(ok).is_ok(), "{ok}: {:?}", url(ok).err());
        }
        for bad in [
            "http://example.test/new?t={title}",
            "1app://x?t={title}",
            "{title}",
            "no-scheme-here",
            " javascript:alert(1)",
        ] {
            assert!(url(bad).is_err(), "{bad} loaded");
        }
    }

    #[test]
    fn placeholder_in_host_is_refused() {
        for bad in [
            "https://{title}/x",
            "https://example.{repo}/x",
            "https://user@{title}",
            "mailto:{title}",
            "{title}://x",
            "https{title}://x",
        ] {
            let e = url(bad).expect_err(bad);
            assert!(e.contains("url:"), "{bad}: {e}");
        }
        assert!(url("https://example.test/{path}").is_ok());
    }

    #[test]
    fn every_preset_loads_and_round_trips() {
        for p in &PRESETS {
            let vars: String = p
                .vars
                .iter()
                .map(|v| [v, " = \"https://h.test\"\n"].concat())
                .collect();
            let body = format!("preset = \"{}\"\n[send.vars]\n{vars}", p.name);
            let c = load(&body).unwrap_or_else(|e| panic!("{}: {e}", p.name));
            let t = &c.sends[0];
            let values: BTreeMap<String, String> = SEND_FIELDS
                .iter()
                .map(|f| ((*f).to_owned(), "a b".to_owned()))
                .collect();
            let got = fill(&t.parts, &values);
            let mut want = p
                .url
                .replace("{title}", "a%20b")
                .replace("{notes}", "a%20b");
            for v in p.vars {
                want = want.replace(&format!("{{var.{v}}}"), "https://h.test");
            }
            assert_eq!(got, want, "{}", p.name);
            if !p.vars.is_empty() {
                let e =
                    load(&format!("label = \"P\"\npreset = \"{}\"\n", p.name)).expect_err(p.name);
                assert!(e.contains("needs vars."), "{e}");
            }
        }
        assert!(load("label = \"P\"\npreset = \"nope\"\n").is_err());
        assert!(load("label = \"P\"\npreset = \"mail\"\nurl = \"https://x/\"\n").is_err());
    }

    #[derive(Deserialize)]
    struct Vector {
        name: String,
        parts: Vec<Part>,
        values: BTreeMap<String, String>,
        expected: String,
    }

    /// The same file `make send-vectors` runs through send.js, so the two cannot drift.
    #[test]
    fn shared_vectors_hold() {
        let text = include_str!("../testdata/send-vectors.json");
        let vectors: Vec<Vector> = serde_json::from_str(text).expect("vectors");
        assert!(vectors.len() >= 8);
        for v in &vectors {
            assert_eq!(fill(&v.parts, &v.values), v.expected, "{}", v.name);
        }
        for p in &PRESETS {
            assert!(
                vectors
                    .iter()
                    .any(|v| v.name == format!("preset {}", p.name)),
                "{}",
                p.name
            );
        }
    }

    /// A hostile title cannot add a parameter, a fragment or a path: the query stays two
    /// keys and every separator in the value is encoded.
    #[test]
    fn hostile_values_stay_in_their_component() {
        let c = url("https://h.test/new?title={title}&body={notes}").expect("load");
        let mut values = BTreeMap::new();
        values.insert(
            "title".to_owned(),
            "a&admin=1#frag?x=/../\n\u{e9}\u{1f600}".to_owned(),
        );
        values.insert("notes".to_owned(), "-rf +".to_owned());
        let got = fill(&c.sends[0].parts, &values);
        let query = got.split_once('?').expect("query").1;
        assert!(!query.contains(['#', '?', '/', '\n', ' ']), "{got}");
        assert_eq!(query.split('&').count(), 2, "{got}");
    }
}
