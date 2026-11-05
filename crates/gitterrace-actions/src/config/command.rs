//! Placeholder parsing and the command rules checked at load.

use std::collections::BTreeMap;

use crate::config::{FIELDS, Piece, REPO_FIELDS, RawCommand, field_parts};

/// A `command` element that does not split into text and placeholders.
#[derive(Debug, thiserror::Error)]
pub enum PieceError {
    #[error("{element:?}: unclosed {{")]
    Unclosed { element: String },
    #[error("{element:?}: write a literal brace as {{{{ or }}}}")]
    Brace { element: String },
}

/// A placeholder rule a `command` breaks.
#[derive(Debug, thiserror::Error)]
pub(super) enum CommandError {
    #[error(
        "command: shell = true cannot take placeholders ({placeholder}). Write command as an array and drop shell"
    )]
    ShellPlaceholder { placeholder: String },
    #[error("command[{index}]: {source}")]
    Piece { index: usize, source: PieceError },
    #[error("command[{index}]: {{{raw}}}: unknown modifier :{modifier}, known: :{known}")]
    UnknownModifier {
        index: usize,
        raw: String,
        modifier: String,
        known: String,
    },
    #[error("command[{index}]: unknown placeholder {{{name}}}, known: {known}, var.NAME")]
    UnknownPlaceholder {
        index: usize,
        name: String,
        known: String,
    },
    #[error("command[{index}]: {{{name}}}: no var {var:?} in [action.vars]")]
    NoVar {
        index: usize,
        name: String,
        var: String,
    },
    #[error("command[0]: the program is literal, {{{raw}}} is not allowed there")]
    ProgramPlaceholder { raw: String },
    #[error(
        "command[{index}]: {element} comes from the repository and could start with `-`. Put a literal \"--\" before it, or give it a literal prefix (\"--flag={element}\")"
    )]
    NeedsGuard { index: usize, element: String },
}

/// The `stdin` value of an action is none of the three modes.
#[derive(Debug, thiserror::Error)]
#[error("stdin = {0:?}: use json, markdown or none")]
pub(super) struct StdinError(pub(super) String);

/// Splits a `command` element into literal text and `{name}` placeholders. `{{` and `}}` are
/// literal braces; a lone brace is an error so a typo never passes as text.
pub fn pieces(element: &str) -> Result<Vec<Piece>, PieceError> {
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
                    return Err(PieceError::Unclosed {
                        element: element.to_owned(),
                    });
                }
                if name.is_empty() || name.contains('{') {
                    return Err(PieceError::Brace {
                        element: element.to_owned(),
                    });
                }
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Field(name));
            }
            '}' => {
                return Err(PieceError::Brace {
                    element: element.to_owned(),
                });
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
) -> Result<(), CommandError> {
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
                return Err(CommandError::ShellPlaceholder { placeholder: p });
            }
            return Ok(());
        }
        RawCommand::Argv(v) => v,
    };
    let mut after_dashdash = false;
    let mut prev_option = false;
    for (i, el) in elements.iter().enumerate() {
        let ps = pieces(el).map_err(|source| CommandError::Piece { index: i, source })?;
        for p in &ps {
            if let Piece::Field(raw) = p {
                let (name, modifier) = field_parts(raw);
                if let Some(m) = modifier
                    && !crate::schema::MODIFIERS.iter().any(|(k, _)| *k == m)
                {
                    let known: Vec<_> = crate::schema::MODIFIERS.iter().map(|(k, _)| *k).collect();
                    return Err(CommandError::UnknownModifier {
                        index: i,
                        raw: raw.clone(),
                        modifier: m.to_owned(),
                        known: known.join(", :"),
                    });
                }
                if !known(name) {
                    return Err(name.strip_prefix("var.").map_or_else(
                        || CommandError::UnknownPlaceholder {
                            index: i,
                            name: name.to_owned(),
                            known: FIELDS.join(", "),
                        },
                        |v| CommandError::NoVar {
                            index: i,
                            name: name.to_owned(),
                            var: v.to_owned(),
                        },
                    ));
                }
                if i == 0 {
                    return Err(CommandError::ProgramPlaceholder { raw: raw.clone() });
                }
            }
        }
        let opens_with_repo_value = matches!(ps.first(),
            Some(Piece::Field(n)) if REPO_FIELDS.contains(&field_parts(n).0));
        // A value right after a literal option (`--notes {markdown}`) is that option's
        // argument, so the parser never reads it as an option.
        if opens_with_repo_value && !after_dashdash && !prev_option {
            return Err(CommandError::NeedsGuard {
                index: i,
                element: el.clone(),
            });
        }
        after_dashdash |= el == "--";
        prev_option = i > 0 && el.starts_with('-') && el != "--" && !el.contains('{');
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piece_errors_keep_their_text() {
        assert_eq!(
            pieces("a{b").expect_err("open").to_string(),
            "\"a{b\": unclosed {"
        );
        assert_eq!(
            pieces("a{}").expect_err("empty").to_string(),
            "\"a{}\": write a literal brace as {{ or }}"
        );
        assert_eq!(
            pieces("a}").expect_err("lone").to_string(),
            "\"a}\": write a literal brace as {{ or }}"
        );
    }

    #[test]
    fn command_errors_keep_their_text() {
        let e = CommandError::UnknownPlaceholder {
            index: 2,
            name: "x".into(),
            known: "a, b".into(),
        };
        assert_eq!(
            e.to_string(),
            "command[2]: unknown placeholder {x}, known: a, b, var.NAME"
        );
        let e = CommandError::NoVar {
            index: 1,
            name: "var.v".into(),
            var: "v".into(),
        };
        assert_eq!(
            e.to_string(),
            "command[1]: {var.v}: no var \"v\" in [action.vars]"
        );
        let e = CommandError::ProgramPlaceholder {
            raw: "title".into(),
        };
        assert_eq!(
            e.to_string(),
            "command[0]: the program is literal, {title} is not allowed there"
        );
        assert_eq!(
            StdinError("x".into()).to_string(),
            "stdin = \"x\": use json, markdown or none"
        );
    }
}
