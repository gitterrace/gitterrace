//! The names a document may use, checked against their sources: every config key it names
//! exists in the schema and every flag in the clap definition, and the other way round, every
//! key has its row in the configuration page and every shown flag its place in the README.

use std::collections::{BTreeMap, BTreeSet};

use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::readme_agents_md::{cli_flags, shown_flags};

const CONFIG: &str = "docs/configuration.md";
/// Flags clap adds to every command, which the command list does not repeat.
const GLOBAL_CLAP: [&str; 2] = ["help", "version"];

/// The names a document may use: config keys from the schema and long flags from clap.
pub(super) struct Keys {
    sections: BTreeSet<&'static str>,
    ids: BTreeSet<&'static str>,
    /// Each key's default as the configuration page shows it, quotes removed.
    defaults: BTreeMap<&'static str, String>,
    flags: BTreeSet<String>,
    /// [`Self::flags`] without the hidden ones, the flags the README must list.
    shown: BTreeSet<String>,
}

impl Keys {
    pub(super) fn load() -> Self {
        let keys = gitterrace_config::keys();
        Self {
            sections: keys.iter().map(|k| k.section).collect(),
            ids: keys.iter().map(|k| k.id).collect(),
            defaults: keys.iter().map(|k| (k.id, k.default.trim_matches('"').to_owned())).collect(),
            flags: cli_flags(),
            shown: shown_flags(),
        }
    }

    /// Names in inline code (`server.addr`, `--public`, `git terrace export --commits`) and in
    /// fenced `toml` blocks under a config section.
    pub(super) fn check(&self, path: &str, text: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let mut toml = false;
        for ev in Parser::new_ext(text, gitterrace_web::markdown_options()) {
            match ev {
                Event::Code(c) => bad.extend(self.span(path, &c)),
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))) => {
                    toml = lang.as_ref() == "toml";
                }
                Event::End(TagEnd::CodeBlock) => toml = false,
                Event::Text(t) if toml => bad.extend(self.toml(path, &t)),
                Event::Text(t) => {
                    for line in t.lines().filter(|l| l.trim_start().starts_with("git terrace")) {
                        bad.extend(self.flags_in(path, line));
                    }
                }
                _ => {}
            }
        }
        bad
    }

    fn span(&self, path: &str, code: &str) -> Vec<String> {
        let code = code.trim();
        if code.starts_with("git terrace") || (code.starts_with("--") && !code.contains(' ')) {
            return self.flags_in(path, code);
        }
        let code = code.split_whitespace().next().unwrap_or(code);
        let Some((section, name)) = code.split_once('.') else {
            return Vec::new();
        };
        let is_file = ["html", "md", "json", "toml", "rs", "css", "js"].contains(&name);
        let is_key = self.sections.contains(section)
            && !is_file
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_lowercase() || c == '_');
        if is_key && !self.ids.contains(code) {
            return vec![format!("{path}: `{code}` is not a config key")];
        }
        Vec::new()
    }

    fn flags_in(&self, path: &str, text: &str) -> Vec<String> {
        text.split_whitespace()
            .filter_map(|w| w.strip_prefix("--"))
            .map(|w| w.split(['=', ',', ')', ']', '`']).next().unwrap_or(w))
            .filter(|f| !f.is_empty() && !f.ends_with('-') && !f.contains('<'))
            .filter(|f| !self.flags.contains(*f))
            .map(|f| format!("{path}: `--{f}` is not a flag of git terrace"))
            .collect()
    }

    /// The other direction: every key has its entry in the configuration page and every flag
    /// its place in the README's command list, so neither reference leaves one out.
    pub(super) fn reverse(&self, texts: &BTreeMap<String, String>) -> Vec<String> {
        let mut bad = Vec::new();
        if let Some(config) = texts.get(CONFIG) {
            for id in &self.ids {
                let (section, name) = id.split_once('.').unwrap_or((id, ""));
                let row = format!("\n- `{name}` (");
                let head = format!("## `[{section}]`");
                let found = config
                    .split("\n## ")
                    .any(|part| format!("## {part}").starts_with(&head) && part.contains(&row));
                if !found {
                    bad.push(format!("{CONFIG}: no row for `{id}`"));
                }
            }
        }
        if let Some(readme) = texts.get("README.md") {
            for f in self.shown.iter().filter(|f| !GLOBAL_CLAP.contains(&f.as_str())) {
                if !readme.contains(&format!("--{f}")) {
                    bad.push(format!("README.md: the command list leaves out `--{f}`"));
                }
            }
        }
        bad
    }

    /// Values prose states as the code's: a key followed by `(default V` or `(V by default)`
    /// must have the default `V`, and an address followed by `by default` must be
    /// [`gitterrace_config::DEFAULT_ADDR`]. A unit after `V` (`64 KiB`) is not compared.
    pub(super) fn defaults(&self, path: &str, text: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let prose = prose(text);
        let mut parts = prose.split('`');
        parts.next();
        while let (Some(code), Some(after)) = (parts.next(), parts.next()) {
            let after: Vec<&str> = after.split_whitespace().take(5).collect();
            let after = after.join(" ");
            let addr = is_addr(code) && after.starts_with("by default");
            let stated = match self.defaults.get(code) {
                Some(want) => stated_default(&after).map(|v| (v, want.as_str())),
                None if addr => Some((code.to_owned(), gitterrace_config::DEFAULT_ADDR)),
                None => None,
            };
            if let Some((v, want)) = stated
                && v != want
            {
                bad.push(format!(
                    "{path}: `{code}` is stated as default {v}, the code says {want}"
                ));
            }
        }
        bad
    }

    fn toml(&self, path: &str, text: &str) -> Vec<String> {
        let mut bad = Vec::new();
        let mut section: Option<&str> = None;
        for line in text.lines().map(str::trim) {
            if let Some(h) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = self.sections.get(h).copied();
                continue;
            }
            let (Some(s), Some((name, _))) = (section, line.split_once('=')) else {
                continue;
            };
            let id = format!("{s}.{}", name.trim());
            if !self.ids.contains(id.as_str()) {
                bad.push(format!("{path}: `{id}` in a toml example is not a config key"));
            }
        }
        bad
    }
}

/// The value in `(default V` or `(V by default` within `after`, the words that follow a key.
fn stated_default(after: &str) -> Option<String> {
    let open = &after[after.find('(')? + 1..];
    let inner = open.split(')').next().unwrap_or(open);
    let value = match inner.strip_prefix("default ") {
        Some(v) => v.split_whitespace().next()?,
        None => inner.strip_suffix(" by default")?.split_whitespace().next()?,
    };
    Some(value.trim_matches(['`', '"', ',']).to_owned())
}

/// An IPv4 `host:port`, the shape of a listen address.
fn is_addr(code: &str) -> bool {
    code.rsplit_once(':').is_some_and(|(host, port)| {
        port.parse::<u16>().is_ok()
            && host.split('.').count() == 4
            && host.split('.').all(|o| o.parse::<u8>().is_ok())
    })
}

/// The prose of `text` with each code span in backticks and code blocks left out, so a
/// sentence reads the same across a soft line break.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut block = false;
    for ev in Parser::new_ext(text, gitterrace_web::markdown_options()) {
        match ev {
            Event::Start(Tag::CodeBlock(_)) => block = true,
            Event::End(TagEnd::CodeBlock) => block = false,
            Event::Code(c) => {
                out.push('`');
                out.push_str(&c.replace('`', ""));
                out.push('`');
            }
            Event::Text(t) if !block => out.push_str(&t.replace('`', "")),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(_) => out.push_str(" `` "),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config default, a limit or the listen address that prose states differently from
    /// the code goes red, and the published pages agree with the code.
    #[test]
    fn prose_that_states_a_wrong_default_fails() {
        let keys = Keys::load();
        for bad in [
            "Text over `limits.highlight_kb` (default 512 KiB) shows plain.",
            "it shows within `watch.rescan_secs`\n(31 by default), the setting",
            "Every request over `server.public_timeout_secs` (default 99) ends.",
            "- It serves on `127.0.0.1:8080` by default, so only you.",
        ] {
            assert_eq!(keys.defaults("x.md", bad).len(), 1, "{bad}");
        }
        for good in [
            "Text over `limits.highlight_kb` (default 256 KiB) shows plain.",
            "within `watch.rescan_secs` (30 by default)",
            "It serves on `127.0.0.1:4242` by default.",
            "`--addr 127.0.0.1:0` for a free port, `limits.blob_lines` (a cap)",
        ] {
            assert!(keys.defaults("x.md", good).is_empty(), "{good}");
        }
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
        let mut stated = 0;
        for doc in ["README.md", "CONTRIBUTING.md"].into_iter().map(str::to_owned).chain(
            crate::out::files_in(&std::path::Path::new(root).join("docs"), "md")
                .expect("docs")
                .into_iter()
                .map(|f| format!("docs/{f}")),
        ) {
            let text = std::fs::read_to_string(format!("{root}/{doc}")).expect("doc");
            assert_eq!(keys.defaults(&doc, &text), Vec::<String>::new());
            stated += prose(&text).matches("by default").count();
        }
        assert!(stated > 0, "the published prose states no default to check");
    }
}
