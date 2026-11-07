//! Symbols of Python from its lines: `def`, `async def` and `class`, nested by indentation.
//!
//! Strings, comments and bracketed continuation lines are skipped, so a `def` inside a
//! docstring or a call's arguments is not one. A function inside a function body is a detail,
//! as in the brace languages.

use crate::symbols::Symbol;

/// Skips string literals and comments: the state carried from one line to the next.
#[derive(Default)]
struct Scan {
    /// The closing quote run of an open triple-quoted string.
    triple: Option<&'static str>,
    /// Open brackets: a line starting inside them continues a statement.
    brackets: usize,
    /// The previous line ended in a backslash.
    joined: bool,
}

impl Scan {
    /// Advances over one line and says whether it starts a logical statement.
    fn line(&mut self, line: &str) -> bool {
        let starts = self.triple.is_none() && self.brackets == 0 && !self.joined;
        let b = line.as_bytes();
        let mut i = 0;
        self.joined = false;
        while i < b.len() {
            if let Some(close) = self.triple {
                match line[i..].find(close) {
                    Some(k) if !escaped(b, i + k) => {
                        i += k + 3;
                        self.triple = None;
                    }
                    Some(k) => i += k + 1,
                    None => return starts,
                }
                continue;
            }
            match b[i] {
                b'#' => break,
                b'(' | b'[' | b'{' => self.brackets += 1,
                b')' | b']' | b'}' => self.brackets = self.brackets.saturating_sub(1),
                b'\\' if i + 1 == b.len() => self.joined = true,
                q @ (b'"' | b'\'') => {
                    if b.get(i + 1) == Some(&q) && b.get(i + 2) == Some(&q) {
                        self.triple = Some(if q == b'"' { "\"\"\"" } else { "'''" });
                        i += 3;
                        continue;
                    }
                    i += 1;
                    while i < b.len() && b[i] != q {
                        i += if b[i] == b'\\' { 2 } else { 1 };
                    }
                }
                _ => {}
            }
            i += 1;
        }
        starts
    }
}

/// Whether the byte at `k` follows an odd run of backslashes.
fn escaped(b: &[u8], k: usize) -> bool {
    b[..k].iter().rev().take_while(|c| **c == b'\\').count() % 2 == 1
}

fn ident_len(s: &str) -> usize {
    s.bytes()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c >= 0x80)
        .count()
}

/// What an open block is: its body decides how a `def` inside it is listed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Class,
    /// A function body: nothing inside it is listed.
    Def,
    /// `if`, `try`, `with`, loops and `match`: transparent, a `def` in it belongs to the block
    /// around it.
    Flow,
}

/// Keywords that open a flow block.
const FLOW: [&str; 11] = [
    "if", "elif", "else", "try", "except", "finally", "with", "for", "while", "match", "case",
];

/// The symbols of a Python source.
pub(crate) fn symbols(src: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut scan = Scan::default();
    let mut blocks: Vec<(usize, Block)> = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let starts = scan.line(line);
        let body = line.trim_start_matches([' ', '\t']);
        if !starts || body.is_empty() || body.starts_with('#') {
            continue;
        }
        let indent = line.len() - body.len();
        while blocks.last().is_some_and(|b| b.0 >= indent) {
            blocks.pop();
        }
        let rest = body.strip_prefix("async ").map_or(body, str::trim_start);
        let (class, rest) = if let Some(r) = rest.strip_prefix("def ") {
            (false, r)
        } else if let Some(r) = rest.strip_prefix("class ") {
            (true, r)
        } else {
            let word = &rest[..ident_len(rest)];
            if FLOW.contains(&word) && !rest[word.len()..].starts_with(['=', '.', '(', '[']) {
                blocks.push((indent, Block::Flow));
            }
            continue;
        };
        let rest = rest.trim_start();
        let len = ident_len(rest);
        let listed = !blocks.iter().any(|b| b.1 == Block::Def);
        let owner = blocks
            .iter()
            .rev()
            .find(|b| b.1 != Block::Flow)
            .map(|b| b.1);
        if len > 0 && listed {
            out.push(Symbol {
                name: rest[..len].to_owned(),
                line: u32::try_from(n + 1).unwrap_or(u32::MAX),
                depth: u8::from(owner == Some(Block::Class)),
            });
        }
        blocks.push((indent, if class { Block::Class } else { Block::Def }));
    }
    out
}
