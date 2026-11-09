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
                    for line in t
                        .lines()
                        .filter(|l| l.trim_start().starts_with("git terrace"))
                    {
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

    /// The other direction: every key has its row in the configuration page and every flag
    /// its place in the README's command list, so neither reference leaves one out.
    pub(super) fn reverse(&self, texts: &BTreeMap<String, String>) -> Vec<String> {
        let mut bad = Vec::new();
        if let Some(config) = texts.get(CONFIG) {
            for id in &self.ids {
                let (section, name) = id.split_once('.').unwrap_or((id, ""));
                let row = format!("| `{name}` |");
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
            for f in self
                .shown
                .iter()
                .filter(|f| !GLOBAL_CLAP.contains(&f.as_str()))
            {
                if !readme.contains(&format!("--{f}")) {
                    bad.push(format!("README.md: the command list leaves out `--{f}`"));
                }
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
                bad.push(format!(
                    "{path}: `{id}` in a toml example is not a config key"
                ));
            }
        }
        bad
    }
}
