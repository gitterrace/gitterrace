//! Placeholder parsing and the command rules checked at load.

use std::collections::BTreeMap;

use crate::config::{FIELDS, Piece, REPO_FIELDS, RawCommand, field_parts};

/// Splits a `command` element into literal text and `{name}` placeholders. `{{` and `}}` are
/// literal braces; a lone brace is an error so a typo never passes as text.
pub fn pieces(element: &str) -> Result<Vec<Piece>, String> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut chars = element.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut name = String::new();
                let mut closed = false;
                for c in chars.by_ref() {
                    if c == '}' {
                        closed = true;
                        break;
                    }
                    name.push(c);
                }
                if !closed {
                    return Err(format!("{element:?}: unclosed {{"));
                }
                if name.is_empty() || name.contains('{') {
                    return Err(format!(
                        "{element:?}: write a literal brace as {{{{ or }}}}"
                    ));
                }
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Field(name));
            }
            '}' => {
                return Err(format!(
                    "{element:?}: write a literal brace as {{{{ or }}}}"
                ));
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    Ok(out)
}

/// The placeholder rules, checked once at load. `vars` is the action's `[action.vars]`, or
/// `None` for a `[[script]]` block, whose `{var.NAME}` is checked against each action using it.
pub(super) fn check_command(
    command: &RawCommand,
    shell: bool,
    vars: Option<&BTreeMap<String, String>>,
) -> Result<(), String> {
    let known = |name: &str| {
        name.strip_prefix("var.").map_or_else(
            || FIELDS.contains(&name),
            |v| vars.is_none_or(|vars| vars.contains_key(v)),
        )
    };
    let elements = match command {
        RawCommand::Shell(s) => {
            // `${HOME}` and awk's `{print}` are shell text; only a real placeholder name is
            // refused, so existing shell commands keep working.
            let named = FIELDS
                .iter()
                .map(|f| format!("{{{f}}}"))
                .chain(std::iter::once("{var.".to_string()))
                .find(|p| s.contains(p.as_str()));
            if shell && let Some(p) = named {
                return Err(format!(
                    "command: shell = true cannot take placeholders ({p}). Write command as an \
                     array and drop shell"
                ));
            }
            return Ok(());
        }
        RawCommand::Argv(v) => v,
    };
    let mut after_dashdash = false;
    let mut prev_option = false;
    for (i, el) in elements.iter().enumerate() {
        let ps = pieces(el).map_err(|why| format!("command[{i}]: {why}"))?;
        for p in &ps {
            if let Piece::Field(raw) = p {
                let (name, modifier) = field_parts(raw);
                if let Some(m) = modifier
                    && !crate::schema::MODIFIERS.iter().any(|(k, _)| *k == m)
                {
                    let known: Vec<_> = crate::schema::MODIFIERS.iter().map(|(k, _)| *k).collect();
                    return Err(format!(
                        "command[{i}]: {{{raw}}}: unknown modifier :{m}, known: :{}",
                        known.join(", :")
                    ));
                }
                if !known(name) {
                    return Err(name.strip_prefix("var.").map_or_else(
                        || {
                            format!(
                                "command[{i}]: unknown placeholder {{{name}}}, known: {}, var.NAME",
                                FIELDS.join(", ")
                            )
                        },
                        |v| format!("command[{i}]: {{{name}}}: no var {v:?} in [action.vars]"),
                    ));
                }
                if i == 0 {
                    return Err(format!(
                        "command[0]: the program is literal, {{{raw}}} is not allowed there"
                    ));
                }
            }
        }
        let opens_with_repo_value = matches!(ps.first(),
            Some(Piece::Field(n)) if REPO_FIELDS.contains(&field_parts(n).0));
        // A value right after a literal option (`--notes {markdown}`) is that option's
        // argument, so the parser never reads it as an option.
        if opens_with_repo_value && !after_dashdash && !prev_option {
            return Err(format!(
                "command[{i}]: {el} comes from the repository and could start with `-`. Put a \
                 literal \"--\" before it, or give it a literal prefix (\"--flag={el}\")"
            ));
        }
        after_dashdash |= el == "--";
        prev_option = i > 0 && el.starts_with('-') && el != "--" && !el.contains('{');
    }
    Ok(())
}
