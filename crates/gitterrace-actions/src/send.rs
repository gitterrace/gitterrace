//! Send to: `[[send]]` targets are URL templates the browser opens.
//!
//! No process, no POST and
//! no token, so gitterrace stays read-only. The server never builds the final URL: it hands
//! the page the template as parts, and the page fills each placeholder with [`fill`]'s rule
//! (`static/send.js` mirrors it, `testdata/send-vectors.json` holds both to it).
//!
//! Every check runs at load, on the template after `{var.NAME}` is substituted: `vars` come
//! from the user's own config, are inserted verbatim, and may therefore form the base URL.
//! Page placeholders are always percent-encoded and may never reach the scheme or the host.

use std::collections::BTreeMap;

use gitterrace_git::percent_encode_segment;
use serde::{Deserialize, Serialize};

use crate::config::{Piece, pieces};

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

/// Placeholders a `lookup` template may hold: only the commit's own ids.
pub const LOOKUP_FIELDS: [&str; 2] = ["sha", "short"];

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
    /// How to search the destination for a commit; `None` when the target has no such search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup: Option<Vec<Part>>,
}

impl Target {
    /// The link that searches this destination for `sha`: `None` without a `lookup`.
    /// Filled with [`fill`]'s rule, so the id is percent-encoded.
    pub fn lookup_url(&self, sha: &str) -> Option<String> {
        let short: String = sha.chars().take(7).collect();
        let values = BTreeMap::from([
            ("sha".to_owned(), sha.to_owned()),
            ("short".to_owned(), short),
        ]);
        self.lookup.as_deref().map(|p| fill(p, &values))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawSend {
    id: String,
    label: String,
    url: String,
    /// Optional template that searches the destination for a commit: `{sha}`, `{short}` and
    /// `{var.NAME}`, under the same scheme rules as `url`.
    #[serde(default)]
    lookup: Option<String>,
    /// `false` keeps the block checked but not offered: how the shipped starter file lists
    /// targets that need settings of yours.
    #[serde(default = "yes")]
    pub(crate) enabled: bool,
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
            Part::Ph(n) => percent_encode_segment(values.get(n).map_or("", String::as_str)),
        })
        .collect()
}

const fn yes() -> bool {
    true
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
    let label = raw.label.as_str();
    if label.trim().is_empty() || label.chars().count() > 80 {
        return Err("label must be 1 to 80 characters".into());
    }
    let parts = template_parts(&raw.vars, "url", &raw.url, &SEND_FIELDS)?;
    let lookup = raw
        .lookup
        .as_deref()
        .map(|t| template_parts(&raw.vars, "lookup", t, &LOOKUP_FIELDS))
        .transpose()?;
    Ok(Target {
        id: raw.id,
        label: raw.label,
        parts,
        lookup,
    })
}

/// Splits `template` into parts: `{var.NAME}` is inserted verbatim, a name in `allowed`
/// stays a placeholder, anything else is refused. `key` names the field in the error.
fn template_parts(
    vars: &BTreeMap<String, String>,
    key: &str,
    template: &str,
    allowed: &[&str],
) -> Result<Vec<Part>, String> {
    let mut parts: Vec<Part> = Vec::new();
    let push_lit = |parts: &mut Vec<Part>, s: &str| match parts.last_mut() {
        Some(Part::Lit(l)) => l.push_str(s),
        _ => parts.push(Part::Lit(s.to_owned())),
    };
    for piece in pieces(template).map_err(|e| format!("{key}: {e}"))? {
        match piece {
            Piece::Text(t) => push_lit(&mut parts, &t),
            Piece::Field(f) => {
                if let Some(v) = f.strip_prefix("var.") {
                    let value = vars
                        .get(v)
                        .ok_or_else(|| format!("{key}: {{{f}}}: no vars.{v}"))?;
                    push_lit(&mut parts, value);
                } else if allowed.contains(&f.as_str()) {
                    parts.push(Part::Ph(f));
                } else {
                    return Err(format!(
                        "{key}: unknown placeholder {{{f}}}, known: {}, var.<name>",
                        allowed.join(", ")
                    ));
                }
            }
        }
    }
    let prefix = match parts.first() {
        Some(Part::Lit(l)) => l.as_str(),
        _ => "",
    };
    check_prefix(key, prefix, parts.len() > 1)?;
    Ok(parts)
}

/// The literal text before the first page placeholder must fix the scheme and, when a
/// placeholder follows, end the host: a value can then only land in the path, query or
/// fragment.
fn check_prefix(key: &str, prefix: &str, has_placeholder: bool) -> Result<(), String> {
    let (scheme, rest) = prefix
        .split_once(':')
        .ok_or_else(|| format!("{key}: the scheme must be literal, before any placeholder"))?;
    let lower = scheme.to_ascii_lowercase();
    let well_formed = lower.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && lower
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-".contains(&b));
    if !well_formed {
        return Err(format!("{key}: bad scheme {scheme:?}"));
    }
    if DENIED_SCHEMES.contains(&lower.as_str()) {
        return Err(format!("{key}: scheme {lower}: is refused"));
    }
    let after = rest.strip_prefix("//").unwrap_or(rest);
    let host_end = after.find(['/', '?', '#']);
    if has_placeholder && host_end.is_none() {
        return Err(format!(
            "{key}: a placeholder may not be part of the scheme or host"
        ));
    }
    if lower == "http" {
        let authority = &after[..host_end.unwrap_or(after.len())];
        let host = authority.rsplit('@').next().unwrap_or_default();
        let host = host.strip_prefix('[').map_or_else(
            || host.split(':').next().unwrap_or_default(),
            |v6| v6.split(']').next().unwrap_or_default(),
        );
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback());
        if !loopback {
            return Err(format!(
                "{key}: http is allowed only for loopback. Use https"
            ));
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

    /// The starter file, with every send switched on: the sends part is plain data.
    fn starter_sends() -> Config {
        let text = Config::STARTER
            .split("# ---- Actions")
            .next()
            .expect("sends part")
            .replace("enabled = false", "enabled = true");
        Config::parse(&text, Path::new("/")).expect("starter sends load")
    }

    #[test]
    fn starter_file_loads_and_hides_the_unset() {
        let c = Config::parse(Config::STARTER, Path::new("/")).expect("starter loads");
        let ids: Vec<&str> = c.sends.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["tendir", "mail"]);
        assert!(c.actions.is_empty(), "nothing offered until you enable it");
        assert!(starter_sends().sends.len() > 6);
    }

    #[test]
    fn disabled_send_is_still_checked() {
        let e = load("label = \"S\"\nenabled = false\nurl = \"javascript:x{title}\"\n")
            .expect_err("bad url");
        assert!(e.contains("refused"), "{e}");
    }

    #[test]
    fn gitlab_urls_keep_each_value_in_its_parameter() {
        let c = starter_sends();
        let mut values = BTreeMap::new();
        values.insert("title".to_owned(), "a&b #1 ?/ + \u{e9}\u{1f600}".to_owned());
        values.insert("notes".to_owned(), "x=1&issue[title]=evil".to_owned());
        for id in [
            "gitlab-issue",
            "gitlab-issue-self-hosted",
            "gitlab-merge-request",
        ] {
            let t = c.sends.iter().find(|t| t.id == id).expect(id);
            let got = fill(&t.parts, &values);
            let query = got.split_once('?').expect("query").1;
            assert!(!query.contains(['#', '?', '/', ' ', '+']), "{id}: {got}");
            let n = query.split('&').count();
            assert_eq!(
                n,
                if id.ends_with("request") { 3 } else { 2 },
                "{id}: {got}"
            );
            assert!(got.contains("a%26b%20%231%20%3F%2F%20%2B%20%C3%A9%F0%9F%98%80"));
        }
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
        // One vector per starter send, by id, so a new entry needs its vector.
        for t in &starter_sends().sends {
            let name = format!("starter {}", t.id);
            assert!(vectors.iter().any(|v| v.name == name), "{name}");
            let v = vectors.iter().find(|v| v.name == name).expect("vector");
            assert_eq!(v.parts, t.parts, "{name}");
            if let Some(lookup) = &t.lookup {
                let name = format!("starter lookup {}", t.id);
                let v = vectors.iter().find(|v| v.name == name).expect(&name);
                assert_eq!(&v.parts, lookup, "{name}");
            }
        }
    }

    #[test]
    fn lookup_allows_only_the_commit_ids() {
        let ok = |l: &str| {
            load(&format!(
                "label = \"S\"\nurl = \"mailto:x\"\nlookup = {l:?}\n"
            ))
        };
        assert!(ok("https://h.test/s?q={sha}&s={short}").is_ok());
        for bad in [
            "https://h.test/s?q={title}",
            "https://h.test/s?q={notes}",
            "https://h.test/s?q={repo}",
            "https://h.test/s?q={path}",
            "https://{sha}.test/s",
            "javascript:x{sha}",
            "http://h.test/s?q={sha}",
            "no-scheme?q={sha}",
        ] {
            let e = ok(bad).expect_err(bad);
            assert!(e.contains("lookup:"), "{bad}: {e}");
        }
        let e = ok("https://h.test/{var.nope}/{sha}").expect_err("var");
        assert!(e.contains("lookup: {var.nope}"), "{e}");
        // `sha` is a url placeholder but `short` is not: the two lists differ on purpose.
        assert!(url("https://h.test/s?q={short}").is_err());
    }

    #[test]
    fn lookup_url_is_filled_and_absent_without_a_lookup() {
        let c = load(
            "label = \"S\"\nurl = \"mailto:x\"\nlookup = \"{var.base}/s?q={sha}&s={short}\"\n\
             vars = { base = \"https://h.test\" }\n",
        )
        .expect("load");
        let t = &c.sends[0];
        assert_eq!(
            t.lookup_url("0123456789abcdef").as_deref(),
            Some("https://h.test/s?q=0123456789abcdef&s=0123456")
        );
        // Whatever arrives is encoded, never spliced.
        assert_eq!(
            t.lookup_url("a&b#c").as_deref(),
            Some("https://h.test/s?q=a%26b%23c&s=a%26b%23c")
        );
        assert_eq!(
            url("mailto:x").expect("load").sends[0].lookup_url("abc"),
            None
        );
    }

    /// Every starter issue target has a commit search, and a lookup holds only the commit's ids.
    #[test]
    fn starter_lookups_use_only_commit_ids_and_cover_issue_targets() {
        for t in &starter_sends().sends {
            if t.id.contains("issue") {
                assert!(t.lookup.is_some(), "{} has no commit search", t.id);
            }
            for part in t.lookup.iter().flatten() {
                if let Part::Ph(name) = part {
                    assert!(LOOKUP_FIELDS.contains(&name.as_str()), "{}: {name}", t.id);
                }
            }
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
