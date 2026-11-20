//! The token walk the brace languages share: the open blocks, and which of them are
//! containers whose declarations an outline lists.
//!
//! A symbol is listed when every block around it is a container: a module, impl, trait,
//! class or namespace body. A function inside a function body is a detail, not an outline
//! entry, so it is skipped with the body.

use crate::symbols::lex::{Dialect, Kind, Tok, tokens};
use crate::symbols::{Symbol, SymbolKind};

#[derive(Clone, Copy)]
struct Open {
    byte: u8,
    container: bool,
    class: bool,
}

pub(crate) struct Walk<'a> {
    pub(crate) src: &'a str,
    pub(crate) toks: Vec<Tok>,
    pub(crate) i: usize,
    stack: Vec<Open>,
    /// The next `{` opens a container body (and a class body when the second is set).
    pub(crate) pending: Option<bool>,
    pub(crate) out: Vec<Symbol>,
}

impl<'a> Walk<'a> {
    pub(crate) fn text(&self, t: Tok) -> &'a str {
        &self.src[t.start as usize..t.end as usize]
    }

    pub(crate) fn tok(&self, k: usize) -> Option<Tok> {
        self.toks.get(k).copied()
    }

    pub(crate) fn word(&self, k: usize) -> Option<&'a str> {
        let t = self.tok(k)?;
        (t.kind == Kind::Ident).then(|| self.text(t))
    }

    pub(crate) fn punct(&self, k: usize) -> Option<u8> {
        match self.tok(k)?.kind {
            Kind::Punct(p) => Some(p),
            _ => None,
        }
    }

    /// Whether every open block is a container: only then is a declaration an outline entry.
    pub(crate) fn at_item_level(&self) -> bool {
        self.stack.iter().all(|o| o.container)
    }

    /// Whether no block is open: Go declares only at the top level.
    pub(crate) const fn at_top_level(&self) -> bool {
        self.stack.is_empty()
    }

    pub(crate) fn in_class_body(&self) -> bool {
        self.stack.last().is_some_and(|o| o.class)
    }

    pub(crate) fn depth(&self) -> u8 {
        u8::from(!self.stack.is_empty())
    }

    /// Lists `name`, a `kind` declared at token `at`, at the current nesting.
    pub(crate) fn emit(&mut self, name: String, at: usize, kind: SymbolKind) {
        let line = self.toks[at].line;
        let depth = self.depth();
        self.out.push(Symbol { name, line, depth, kind: Some(kind) });
    }

    /// The index past a balanced `open ... close` run starting at `k`, which holds `open`.
    pub(crate) fn skip_balanced(&self, k: usize, open: u8, close: u8) -> usize {
        let mut depth = 0usize;
        let mut j = k;
        while let Some(t) = self.tok(j) {
            if let Kind::Punct(p) = t.kind {
                if p == open {
                    depth += 1;
                } else if p == close
                    && !(close == b'>'
                        && self.punct(j.wrapping_sub(1)) == Some(b'-')
                        && self.toks[j - 1].end == t.start)
                {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return j + 1;
                    }
                } else if open == b'<' && matches!(p, b'{' | b';') {
                    return j;
                }
            }
            j += 1;
        }
        j
    }

    pub(crate) fn bracket(&mut self, p: u8) {
        match p {
            b'{' => {
                let pending = self.pending.take();
                self.stack.push(Open {
                    byte: p,
                    container: pending.is_some(),
                    class: pending == Some(true),
                });
            }
            // A header's own brackets (`impl AsRef<[u8]>`, `where F: Fn()`) keep the body pending.
            b'(' | b'[' => {
                self.stack.push(Open { byte: p, container: false, class: false });
            }
            b'}' | b')' | b']' => {
                let want = match p {
                    b'}' => b'{',
                    b')' => b'(',
                    _ => b'[',
                };
                // A stray closer pops nothing rather than unwinding a container.
                if self.stack.last().is_some_and(|o| o.byte == want) {
                    self.stack.pop();
                }
            }
            // `;` ends a bodiless item, but not inside a header's `[T; N]`.
            b';' if self.stack.last().is_none_or(|o| o.byte == b'{') => self.pending = None,
            _ => {}
        }
    }
}

pub(crate) fn symbols(src: &str, dialect: Dialect) -> Vec<Symbol> {
    let mut w = Walk {
        src,
        toks: tokens(src, dialect),
        i: 0,
        stack: Vec::new(),
        pending: None,
        out: Vec::new(),
    };
    match dialect {
        Dialect::Rust => crate::symbols::rust_go::rust(&mut w),
        Dialect::Go => crate::symbols::rust_go::go(&mut w),
        Dialect::Script => crate::symbols::script::script(&mut w),
    }
    // A declarator list is read ahead of the walk, so its later names land before the class
    // members between them: a stable sort by line restores source order.
    w.out.sort_by_key(|s| s.line);
    w.out
}
