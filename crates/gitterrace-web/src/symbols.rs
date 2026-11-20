//! The symbol outline of a source file: its functions, types, classes and methods, each with
//! its line, for the "On this page" list of the file page.
//!
//! The language comes from the highlighter's detector, so the outline and the colours agree.
//! Rust, Go, Python and TS/JS have a small scanner: exact about nesting and members, and
//! under 2 ms on a 5000-line file. Every other language takes the declarations its grammar
//! names (`scopes`), recorded by the highlight pass the page already makes, so no language
//! pays a second parse.

mod lex;
mod python;
mod rust_go;
pub(crate) mod scopes;
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
    /// What it declares, from the keyword the scanner matched. `None` when no keyword says.
    pub(crate) kind: Option<SymbolKind>,
}

/// What a symbol declares: the outline shows it as a short label and speaks the full word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SymbolKind {
    Function,
    /// A function in a class, trait or impl body, or a Go function with a receiver.
    Method,
    Struct,
    Enum,
    Union,
    Trait,
    Impl,
    Class,
    Interface,
    Type,
    Module,
    Namespace,
    Macro,
}

impl SymbolKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Function => "fn",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Union => "union",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::Class => "class",
            Self::Interface => "iface",
            Self::Type => "type",
            Self::Module => "mod",
            Self::Namespace => "ns",
            Self::Macro => "macro",
        }
    }

    pub(crate) const fn spoken(self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Union => "union",
            Self::Trait => "trait",
            Self::Impl => "implementation",
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Type => "type",
            Self::Module => "module",
            Self::Namespace => "namespace",
            Self::Macro => "macro",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scanner {
    Brace(Dialect),
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

/// Whether a file's language has a scanner, so its highlight pass need not record symbols.
pub(crate) fn scanned(path: &str, first_line: &str) -> bool {
    crate::highlight::language(path, first_line).is_some_and(|l| scanner(l).is_some())
}

/// Whether the highlight pass of a file records the symbols its grammar names: a language
/// without a scanner, and not prose or data whose grammar reuses `entity.name` for headers.
pub(crate) fn scoped(path: &str, first_line: &str) -> bool {
    crate::highlight::language(path, first_line).is_some_and(|l| {
        scanner(l).is_none()
            && !matches!(
                l,
                "Email" | "Command Help" | "Manpage" | "MediaWiki" | "Textile" | "BibTeX"
            )
    })
}

/// The symbols of `text`, a file at `path`, in source order, by its scanner. Empty for a
/// language without one.
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
/// the page. `named` are the symbols the highlight pass recorded, used when the language has
/// no scanner.
pub(crate) fn outline(
    path: &str,
    text: &str,
    lines: usize,
    named: Vec<Symbol>,
) -> Vec<OutlineItem> {
    let shown =
        text.match_indices('\n').nth(lines.saturating_sub(1)).map_or(text, |(i, _)| &text[..i]);
    let shown = if lines == 0 { "" } else { shown };
    let first = shown.lines().next().unwrap_or("");
    let found = if scanned(path, first) { symbols(path, shown) } else { named };
    found
        .into_iter()
        .map(|s| OutlineItem {
            level: 2 + s.depth.min(1),
            id: format!("L{}", s.line),
            text: s.name,
            chrome: true,
            kind: s.kind,
        })
        .collect()
}

#[cfg(test)]
mod tests;
