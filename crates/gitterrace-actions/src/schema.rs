//! The documented shape of the actions contract.
//!
//! Payload fields, `actions.toml` keys,
//! environment variables and kinds. `cargo xtask generate` renders `docs/actions.md` and the
//! README from these tables, and the tests below fail when a table and the code disagree, so a
//! change to a field, key or kind cannot ship without its documentation.

use crate::config::Kind;

/// One documented field or key.
pub struct Item {
    pub name: &'static str,
    pub ty: &'static str,
    /// Always present (payload) or required (config).
    pub required: bool,
    /// Config default, empty for payload fields.
    pub default: &'static str,
    pub doc: &'static str,
}

const fn item(
    name: &'static str,
    ty: &'static str,
    required: bool,
    default: &'static str,
    doc: &'static str,
) -> Item {
    Item {
        name,
        ty,
        required,
        default,
        doc,
    }
}

/// Every kind an action can be offered on, with what the selection is.
/// What an action is, in one plain sentence, for the website and docs.
pub const ACTION_SUMMARY: &str = "Run a program of yours on a selection, with no shell between: a task CLI, an editor, a script.";

pub const KINDS: [(Kind, &str); 8] = [
    (Kind::Line, "A line or a range of lines in a file."),
    (Kind::File, "A whole file."),
    (Kind::Commit, "A commit."),
    (Kind::Hunk, "One hunk of a diff."),
    (Kind::Blame, "One line of a blame view."),
    (Kind::SearchHit, "A code or commit search hit."),
    (
        Kind::Insight,
        "A row of an insights page, such as a hotspot.",
    ),
    (Kind::Page, "The page being shown."),
];

/// The kebab-case name of `kind`, as written in `actions.toml` and the payload.
pub fn kind_name(kind: Kind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Payload v1, the JSON a script reads on stdin. `required` means always present.
pub const PAYLOAD_FIELDS: [Item; 16] = [
    item("v", "integer", true, "", "Payload version, `1`."),
    item("action", "string", true, "", "The action's `id`."),
    item(
        "kind",
        "string",
        true,
        "",
        "What was selected, one of the kinds below.",
    ),
    item(
        "repo",
        "object",
        true,
        "",
        "`name`, `root` (absolute path) and `id` (stable repository id, absent for a repository without commits).",
    ),
    item(
        "rev",
        "string",
        true,
        "",
        "The revision the page was showing, as named in the URL.",
    ),
    item(
        "sha",
        "string",
        true,
        "",
        "The full commit id `rev` resolves to.",
    ),
    item(
        "path",
        "string",
        false,
        "",
        "Repository-relative path, when the selection is in a file.",
    ),
    item(
        "lines",
        "object",
        false,
        "",
        "`start` and `end`, 1-based and inclusive.",
    ),
    item(
        "text",
        "string",
        false,
        "",
        "The selected text, read from git. Capped at 64 KiB or 400 lines.",
    ),
    item(
        "text_truncated",
        "boolean",
        false,
        "",
        "Set with `text`: true when a cap cut it.",
    ),
    item(
        "query",
        "string",
        false,
        "",
        "The search query, for a search hit.",
    ),
    item(
        "permalink",
        "string",
        true,
        "",
        "`gitterrace://<repo-id>/<sha>/<path>#L<a>-L<b>`, or the `/go` URL with `links.style = \"go\"`. Survives clones and renames.",
    ),
    item(
        "title",
        "string",
        true,
        "",
        "A short label such as `guard.rs:44-60 @ 4f0c2e9`.",
    ),
    item(
        "commit",
        "object",
        false,
        "",
        "`subject`, `author` and `date` (RFC 3339) of `sha`.",
    ),
    item(
        "markdown",
        "string",
        true,
        "",
        "The same text the Copy as markdown built-in produces.",
    ),
    item(
        "vars",
        "object",
        true,
        "",
        "The action's `[action.vars]`, possibly empty. Set from `actions.toml` only.",
    ),
];

/// The environment a script gets, besides what the action lists in `env`.
pub const ENV_VARS: [(&str, &str); 11] = [
    ("GITTERRACE_PAYLOAD_VERSION", "The payload version."),
    ("GITTERRACE_ACTION", "The action's `id`."),
    ("GITTERRACE_KIND", "The kind."),
    ("GITTERRACE_REPO", "The repository name."),
    ("GITTERRACE_ROOT", "The repository root."),
    ("GITTERRACE_SHA", "The commit id."),
    ("GITTERRACE_TITLE", "The title."),
    ("GITTERRACE_URL", "The permalink."),
    ("GITTERRACE_PATH", "The path, when there is one."),
    (
        "GITTERRACE_LINE_START",
        "First line, when there is a range.",
    ),
    ("GITTERRACE_LINE_END", "Last line, when there is a range."),
];

/// Placeholders a `command` element may hold. `repo` marks values read from the browsed
/// repository, which the loader keeps from opening an argument.
pub const PLACEHOLDERS: [(&str, bool, &str); 12] = [
    ("title", true, "The title."),
    ("markdown", true, "The markdown text."),
    ("text", true, "The selected text, or empty."),
    ("path", true, "The repository-relative path, or empty."),
    ("line_start", false, "First line, or empty."),
    ("line_end", false, "Last line, or empty."),
    ("sha", false, "The full commit id."),
    ("url", false, "The permalink."),
    ("repo", false, "The repository name."),
    ("root", false, "The repository root."),
    ("kind", false, "The kind."),
    (
        "var.<name>",
        false,
        "A value from `[action.vars]`. An undefined name is a load error.",
    ),
];

/// Modifiers any placeholder may take as `{name:modifier}`; another is a load error.
pub const MODIFIERS: [(&str, &str); 1] = [(
    "url",
    "Percent-encodes the value for a URL query value: letters, digits and `-._~` stay, every \
     other byte (`&`, `=`, `?`, `#`, `/`, `+`, space, newline, non-ASCII) becomes `%XX`, so \
     the value cannot leave its parameter. The caps count the encoded length.",
)];

/// Top-level keys of `actions.toml`.
pub const FILE_KEYS: [Item; 4] = [
    item("version", "integer", true, "", "Must be `1`."),
    item(
        "script",
        "array of tables",
        false,
        "",
        "`[[script]]` blocks: a command line written once for several actions.",
    ),
    item(
        "action",
        "array of tables",
        false,
        "",
        "`[[action]]` blocks: what the menu offers.",
    ),
    item(
        "send",
        "array of tables",
        false,
        "",
        "`[[send]]` blocks: Send to targets, URLs the browser opens.",
    ),
];

/// Keys of a `[[send]]` block.
pub const SEND_KEYS: [Item; 6] = [
    item(
        "id",
        "string",
        true,
        "",
        "`[a-z0-9-]`, 1 to 32 characters, unique among sends.",
    ),
    item(
        "label",
        "string",
        true,
        "",
        "Text in the Send window, 1 to 80 characters.",
    ),
    item(
        "url",
        "string",
        true,
        "",
        "The URL template. The scheme and host are literal. Placeholders (below) fill the \
         path, query or fragment, each percent-encoded.",
    ),
    item(
        "lookup",
        "string",
        false,
        "",
        "A URL template that searches the destination for a commit. The commit page links \
         to it as \"Find this commit in ...\". Nothing is fetched. Same scheme rules as \
         `url`. Only `{sha}` (full id), `{short}` (first 7 characters) and `{var.<name>}` \
         are allowed.",
    ),
    item(
        "enabled",
        "boolean",
        false,
        "`true`",
        "`false` checks the block but does not offer it. The starter file ships the ones that need your settings this way.",
    ),
    item(
        "vars",
        "table of strings",
        false,
        "`{}`",
        "`[send.vars]`: settings, inserted verbatim as `{var.<name>}`, one line each, at most \
         4 KiB.",
    ),
];

/// Placeholders a `[[send]]` url may hold; the page fills them, each percent-encoded.
pub const SEND_PLACEHOLDERS: [(&str, &str); 8] = [
    ("title", "The note's first line, or the selection's name."),
    (
        "notes",
        "The rest of the note, the quoted selection and the permalink.",
    ),
    ("permalink", "The permalink, in `links.style`."),
    ("path", "The repository-relative path, or empty."),
    ("lines", "The line range, like `L3-L7`, or empty."),
    ("sha", "The full commit id."),
    ("repo", "The repository name."),
    (
        "var.<name>",
        "A value from `[send.vars]`, inserted verbatim, so it may hold the base URL.",
    ),
];

/// Keys of a `[[script]]` block.
pub const SCRIPT_KEYS: [Item; 6] = [
    item(
        "id",
        "string",
        true,
        "",
        "`[a-z0-9-]`, 1 to 32 characters, unique. A script cannot name another script.",
    ),
    item(
        "command",
        "array or string",
        true,
        "",
        "The argv, run with no shell. `argv[0]` is literal, resolved once at load. Other elements may hold placeholders (below), each filled in as exactly one argument. One string when `shell = true`, which takes no placeholders.",
    ),
    item(
        "stdin",
        "string",
        false,
        "`json`",
        "What the program reads: `json` (the payload), `markdown` (the `markdown` field alone) or `none`.",
    ),
    item(
        "cwd",
        "string",
        false,
        "`home`",
        "`home`, `repo` (the repository root) or an absolute folder.",
    ),
    item(
        "timeout",
        "string",
        false,
        "`10s`",
        "Like `10s` or `500ms`, at most `120s`.",
    ),
    item(
        "shell",
        "boolean",
        false,
        "`false`",
        "Run `command` as `sh -c \"<string>\"`. A placeholder in it is a load error. Read the environment instead.",
    ),
];

/// Keys of an `[[action]]` block.
pub const ACTION_KEYS: [Item; 14] = [
    item(
        "id",
        "string",
        true,
        "",
        "`[a-z0-9-]`, 1 to 32 characters, unique.",
    ),
    item(
        "label",
        "string",
        true,
        "",
        "Menu text, 1 to 80 characters.",
    ),
    item(
        "enabled",
        "boolean",
        false,
        "`true`",
        "`false` checks the block but does not offer it, and its program may be missing. The starter file ships the examples this way.",
    ),
    item(
        "kinds",
        "array of strings",
        true,
        "",
        "Where the action is offered, from the kinds below. An unknown kind is an error.",
    ),
    item(
        "command",
        "array or string",
        false,
        "",
        "As in a script. Set exactly one of `command` and `script`.",
    ),
    item(
        "script",
        "string",
        false,
        "",
        "The `id` of a `[[script]]`. `shell` is not allowed beside it.",
    ),
    item(
        "cwd",
        "string",
        false,
        "`home`",
        "As in a script. Overrides the script's.",
    ),
    item(
        "timeout",
        "string",
        false,
        "`10s`",
        "As in a script. Overrides the script's.",
    ),
    item(
        "output",
        "string",
        false,
        "`toast`",
        "`toast` shows the first output line, `ignore` shows failures only.",
    ),
    item(
        "stdin",
        "string",
        false,
        "`json`",
        "As in a script. Overrides the script's.",
    ),
    item("shell", "boolean", false, "`false`", "As in a script."),
    item(
        "key",
        "string",
        false,
        "",
        "One letter or digit: a shortcut inside the menu, and on the page press `a` then the key (`a t`). Not `p`, `m` or `s`, and unique among actions; either is a load error.",
    ),
    item(
        "env",
        "array of strings",
        false,
        "`[]`",
        "Variables passed through from gitterrace's environment. `GITTERRACE_*` names are refused.",
    ),
    item(
        "vars",
        "table of strings",
        false,
        "`{}`",
        "`[action.vars]`: fixed settings, lowercase names `[a-z0-9_]{1,32}`, at most 32, each value at most 4 KiB. They reach the script as `GITTERRACE_VAR_<NAME>`, as `payload.vars` and as `{var.<name>}` in `command`.",
    ),
];

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use crate::config::{Config, Kind};
    use crate::payload::{CommitInfo, Lines, Payload, RepoInfo};
    use crate::schema::{
        ACTION_KEYS, ENV_VARS, FILE_KEYS, Item, KINDS, PAYLOAD_FIELDS, PLACEHOLDERS, SCRIPT_KEYS,
        SEND_KEYS, SEND_PLACEHOLDERS, kind_name,
    };

    fn names(items: &[Item]) -> BTreeSet<String> {
        items.iter().map(|i| i.name.to_owned()).collect()
    }

    /// Names a serde or toml "expected one of `a`, `b`" message lists.
    fn expected(message: &str) -> BTreeSet<String> {
        let tail = message.split("expected one of").nth(1).unwrap_or("");
        tail.split('`')
            .enumerate()
            .filter(|(i, _)| i % 2 == 1)
            .map(|(_, s)| s.to_owned())
            .collect()
    }

    fn payload(full: bool) -> Payload {
        Payload {
            v: 1,
            action: "a".into(),
            kind: Kind::Line,
            repo: RepoInfo {
                name: "r".into(),
                root: "/r".into(),
                id: Some("abc".into()),
            },
            rev: "HEAD".into(),
            sha: "0".repeat(40),
            path: full.then(|| "p".into()),
            lines: full.then_some(Lines { start: 1, end: 2 }),
            text: full.then(|| "t".into()),
            text_truncated: full.then_some(false),
            query: full.then(|| "q".into()),
            permalink: "u".into(),
            title: "t".into(),
            commit: full.then(|| CommitInfo {
                subject: "s".into(),
                author: "a".into(),
                date: "d".into(),
            }),
            markdown: "m".into(),
            vars: std::collections::BTreeMap::new(),
        }
    }

    fn keys_of(p: &Payload) -> BTreeSet<String> {
        let v = serde_json::to_value(p).expect("json");
        v.as_object().expect("object").keys().cloned().collect()
    }

    #[test]
    fn payload_table_is_the_payload() {
        assert_eq!(keys_of(&payload(true)), names(&PAYLOAD_FIELDS));
        let absent_when_unset: BTreeSet<String> = names(&PAYLOAD_FIELDS)
            .difference(&keys_of(&payload(false)))
            .cloned()
            .collect();
        let optional: BTreeSet<String> = PAYLOAD_FIELDS
            .iter()
            .filter(|f| !f.required)
            .map(|f| f.name.to_owned())
            .collect();
        assert_eq!(absent_when_unset, optional);
    }

    #[test]
    fn env_table_is_the_environment() {
        let env: BTreeSet<String> = payload(true)
            .env()
            .keys()
            .map(|k| (*k).to_owned())
            .collect();
        let doc: BTreeSet<String> = ENV_VARS.iter().map(|(k, _)| (*k).to_owned()).collect();
        assert_eq!(env, doc);
    }

    #[test]
    fn kinds_table_is_the_kind_enum() {
        let doc: BTreeSet<String> = KINDS.iter().map(|(k, _)| kind_name(*k)).collect();
        let err = serde_json::from_str::<Kind>("\"nope\"").expect_err("unknown kind");
        assert_eq!(doc, expected(&err.to_string()));
        assert_eq!(doc.len(), KINDS.len(), "a kind is listed twice");
    }

    fn parse_error(toml: &str) -> String {
        match Config::parse(toml, Path::new("/")) {
            Ok(_) => String::new(),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn key_tables_are_the_config_keys() {
        let top = parse_error("version = 1\nzzz = 1\n");
        assert_eq!(expected(&top), names(&FILE_KEYS), "{top}");
        let script = parse_error(
            "version = 1\n[[script]]\nid = \"s\"\ncommand = [\"/bin/echo\"]\nzzz = 1\n",
        );
        assert_eq!(expected(&script), names(&SCRIPT_KEYS), "{script}");
        let action = parse_error(
            "version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\n\
             command = [\"/bin/echo\"]\nzzz = 1\n",
        );
        assert_eq!(expected(&action), names(&ACTION_KEYS), "{action}");
        let send = parse_error("version = 1\n[[send]]\nid = \"s\"\nzzz = 1\n");
        assert_eq!(expected(&send), names(&SEND_KEYS), "{send}");
    }

    #[test]
    fn placeholder_table_is_the_placeholders() {
        let p = payload(true);
        for (name, _, _) in &PLACEHOLDERS[..11] {
            assert!(p.placeholder(name).is_some(), "{name}");
        }
        let doc: Vec<&str> = PLACEHOLDERS[..11].iter().map(|(n, _, _)| *n).collect();
        assert_eq!(doc, crate::config::FIELDS);
        let repo: Vec<&str> = PLACEHOLDERS.iter().filter(|p| p.1).map(|p| p.0).collect();
        assert_eq!(repo, crate::config::REPO_FIELDS);
    }

    #[test]
    fn send_placeholder_table_is_the_fields() {
        let doc: Vec<&str> = SEND_PLACEHOLDERS[..7].iter().map(|(n, _)| *n).collect();
        assert_eq!(doc, crate::send::SEND_FIELDS);
    }

    /// The limits the tables state are the limits the loader enforces.
    #[test]
    fn stated_limits_hold() {
        let head = "version = 1\n[[action]]\nid = \"a\"\nlabel = \"A\"\nkinds = [\"line\"]\n\
                    command = [\"/bin/echo\"]\n";
        assert!(Config::parse(&format!("{head}timeout = \"120s\"\n"), Path::new("/")).is_ok());
        assert!(Config::parse(&format!("{head}timeout = \"121s\"\n"), Path::new("/")).is_err());
        let vars = |n: usize| {
            let mut rows = String::new();
            for i in 0..n {
                rows += &["v", &i.to_string(), " = \"x\"\n"].concat();
            }
            format!("{head}[action.vars]\n{rows}")
        };
        assert!(Config::parse(&vars(32), Path::new("/")).is_ok());
        assert!(Config::parse(&vars(33), Path::new("/")).is_err());
        let long = format!("{head}[action.vars]\nv = \"{}\"\n", "x".repeat(4097));
        assert!(Config::parse(&long, Path::new("/")).is_err());
    }
}
