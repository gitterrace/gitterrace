//! The symbol outline of a source file: its functions, types, classes and methods, each with
//! its line, for the "On this page" list of the file page.
//!
//! The language comes from the highlighter's detector, so the outline and the colours agree.
//! Each language has a small scanner rather than a grammar: a full syntect parse costs 100 to
//! 400 ms on a 5000-line file, a scan under 2 ms. Strings and comments are skipped first, so a
//! keyword inside one is never read as a declaration.

mod lex;
mod python;
mod rust_go;
mod script;
mod walk;

use crate::markdown::OutlineItem;
use crate::symbols::lex::Dialect;

/// One declaration: its name, 1-based line and nesting (0 top level, 1 inside a container
/// such as a class or impl).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Symbol {
    pub(crate) name: String,
    pub(crate) line: u32,
    pub(crate) depth: u8,
}

/// Which scanner reads a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scanner {
    /// Tokens and brackets, in one of the brace dialects.
    Brace(Dialect),
    /// Lines and indentation.
    Python,
}

/// The scanner for a highlighter grammar name, or `None` for a language without one.
fn scanner(language: &str) -> Option<Scanner> {
    match language {
        "Rust" => Some(Scanner::Brace(Dialect::Rust)),
        "Go" => Some(Scanner::Brace(Dialect::Go)),
        "TypeScript" | "TypeScriptReact" | "JavaScript" | "JavaScript (Babel)" => {
            Some(Scanner::Brace(Dialect::Script))
        }
        "Python" => Some(Scanner::Python),
        _ => None,
    }
}

/// The symbols of `text`, a file at `path`, in source order. Empty for a language without a
/// scanner.
pub(crate) fn symbols(path: &str, text: &str) -> Vec<Symbol> {
    let first = text.lines().next().unwrap_or("");
    let Some(language) = crate::highlight::language(path, first) else {
        return Vec::new();
    };
    match scanner(language) {
        Some(Scanner::Brace(d)) => walk::symbols(text, d),
        Some(Scanner::Python) => python::symbols(text),
        None => Vec::new(),
    }
}

/// The outline entries of a file: each symbol on a line the page shows (the first `lines`),
/// linked to that line's anchor. Top-level symbols are level 2, members level 3, the two
/// levels the outline lists. Only the shown lines are read, so a cut file costs no more than
/// the page.
pub(crate) fn outline(path: &str, text: &str, lines: usize) -> Vec<OutlineItem> {
    let shown = text
        .match_indices('\n')
        .nth(lines.saturating_sub(1))
        .map_or(text, |(i, _)| &text[..i]);
    let shown = if lines == 0 { "" } else { shown };
    symbols(path, shown)
        .into_iter()
        .map(|s| OutlineItem {
            level: 2 + s.depth.min(1),
            id: format!("L{}", s.line),
            text: s.name,
            chrome: true,
        })
        .collect()
}

#[cfg(test)]
mod tests;
