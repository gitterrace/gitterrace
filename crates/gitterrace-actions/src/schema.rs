//! The documented shape of the actions contract.
//!
//! Payload fields, `actions.toml` keys,
//! environment variables and kinds. `cargo xtask generate` renders `docs/actions.md` and the
//! README from these tables, and the tests below fail when a table and the code disagree, so a
//! change to a field, key or kind cannot ship without its documentation.

use crate::config::Kind;
use crate::numbers;

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
pub const ACTION_SUMMARY: &str = "Run a program of yours on a selection, with no shell between: a \
    task CLI, an editor, a script.";

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
        "`name`, `root` (absolute path) and `id` (stable repository id, absent for a repository \
        without commits).",
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
        concat!(
            "The selected text, read from git. Capped at ",
            numbers::text_kib!(),
            " KiB or ",
            numbers::text_lines!(),
            " lines."
        ),
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
        "`gitterrace://<repo-id>/<sha>/<path>#L<a>-L<b>`, or the `/go` URL with `links.style = \
        \"go\"`. Survives clones and renames.",
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
        "`false` checks the block but does not offer it. The starter file ships the ones that need \
        your settings this way.",
    ),
    item(
        "vars",
        "table of strings",
        false,
        "`{}`",
        concat!(
            "`[send.vars]`: settings, inserted verbatim as `{var.<name>}`, one line each, at \
             most ",
            numbers::var_kib!(),
            " KiB."
        ),
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
        "The argv, run with no shell. `argv[0]` is literal, resolved once at load. Other elements \
        may hold placeholders (below), each filled in as exactly one argument. One string when \
        `shell = true`, which takes no placeholders.",
    ),
    item(
        "stdin",
        "string",
        false,
        "`json`",
        "What the program reads: `json` (the payload), `markdown` (the `markdown` field alone) or \
        `none`.",
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
        concat!(
            "Like `10s` or `500ms`, at most `",
            numbers::timeout_secs!(),
            "s`."
        ),
    ),
    item(
        "shell",
        "boolean",
        false,
        "`false`",
        "Run `command` as `sh -c \"<string>\"`. A placeholder in it is a load error. Read the \
        environment instead.",
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
        "`false` checks the block but does not offer it, and its program may be missing. The \
        starter file ships the examples this way.",
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
        "One letter or digit: a shortcut inside the menu, and on the page press `a` then the key \
        (`a n`). Not a menu key (`p`, `m`, `s`) or a page key (`g`, `t`, `y`, `b`, `j`, `k`), and \
        unique among actions. Either is a load error.",
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
        concat!(
            "`[action.vars]`: fixed settings, lowercase names `[a-z0-9_]{1,",
            numbers::max_vars!(),
            "}`, at most ",
            numbers::max_vars!(),
            ", each value at most ",
            numbers::var_kib!(),
            " KiB. They reach the script as `GITTERRACE_VAR_<NAME>`, as `payload.vars` \
             and as `{var.<name>}` in `command`."
        ),
    ),
];

#[cfg(test)]
mod tests;
