//! Rust and Go declarations from the shared token walk.

use crate::symbols::SymbolKind;
use crate::symbols::lex::Kind;
use crate::symbols::walk::Walk;

/// Rust: `fn`, `struct`, `enum`, `union`, `trait`, `type`, `mod`, `impl`, `macro_rules!`.
pub(crate) fn rust(w: &mut Walk<'_>) {
    while w.i < w.toks.len() {
        let i = w.i;
        let t = w.toks[i];
        w.i += 1;
        let Kind::Ident = t.kind else {
            if let Kind::Punct(p) = t.kind {
                w.bracket(p);
            }
            continue;
        };
        if !w.at_item_level() {
            continue;
        }
        let named = match w.text(t) {
            "fn" if w.in_class_body() => Some(SymbolKind::Method),
            "fn" => Some(SymbolKind::Function),
            "struct" => Some(SymbolKind::Struct),
            "enum" => Some(SymbolKind::Enum),
            "union" => Some(SymbolKind::Union),
            "type" => Some(SymbolKind::Type),
            "trait" => {
                w.pending = Some(true);
                Some(SymbolKind::Trait)
            }
            "mod" => {
                w.pending = Some(false);
                Some(SymbolKind::Module)
            }
            "macro_rules" if w.punct(i + 1) == Some(b'!') => {
                if let Some(name) = w.word(i + 2) {
                    w.emit(name.to_owned(), i + 2, SymbolKind::Macro);
                }
                None
            }
            "impl" if item_start(w, i) => {
                rust_impl(w, i);
                None
            }
            _ => None,
        };
        // `union` is also an ordinary identifier: a type needs a name right after it.
        if let Some(kind) = named
            && let Some(name) = w.word(i + 1)
        {
            w.emit(name.trim_start_matches("r#").to_owned(), i + 1, kind);
        }
    }
}

/// Whether the token at `k` starts an item: after a block, a statement, an attribute or a
/// visibility or `unsafe`. `impl` in a type position (`-> impl Trait`) is not one.
fn item_start(w: &Walk<'_>, k: usize) -> bool {
    let Some(prev) = k.checked_sub(1).and_then(|p| w.tok(p)) else {
        return true;
    };
    match prev.kind {
        Kind::Punct(p) => {
            matches!(p, b'{' | b'}' | b';' | b']') || p == b')' && pub_paren(w, k - 1)
        }
        Kind::Ident => matches!(w.text(prev), "pub" | "unsafe" | "default"),
        Kind::Lit => false,
    }
}

/// Whether the `)` at `k` closes `pub(...)`.
fn pub_paren(w: &Walk<'_>, k: usize) -> bool {
    (1..=4).any(|back| {
        k.checked_sub(back + 1)
            .is_some_and(|o| w.punct(o + 1) == Some(b'(') && w.word(o) == Some("pub"))
    })
}

/// `impl<G> Trait for Type where ... {`: listed as `Trait for Type` of kind impl, a container.
fn rust_impl(w: &mut Walk<'_>, at: usize) {
    let mut k = at + 1;
    if w.punct(k) == Some(b'<') {
        k = w.skip_balanced(k, b'<', b'>');
    }
    let from = k;
    let mut angle = 0usize;
    while let Some(t) = w.tok(k) {
        match t.kind {
            Kind::Punct(b'{' | b';') if angle == 0 => break,
            Kind::Punct(b'<') => angle += 1,
            Kind::Punct(b'>') => angle = angle.saturating_sub(1),
            Kind::Ident if angle == 0 && w.text(t) == "where" => break,
            _ => {}
        }
        k += 1;
    }
    if k == from {
        return;
    }
    let (a, b) = (w.toks[from].start as usize, w.toks[k - 1].end as usize);
    let label: String = w.src[a..b].split_whitespace().collect::<Vec<_>>().join(" ");
    w.emit(label, at, SymbolKind::Impl);
    w.pending = Some(true);
}

/// Go: `func`, methods as `Type.Name`, and `type` alone or in a group.
pub(crate) fn go(w: &mut Walk<'_>) {
    while w.i < w.toks.len() {
        let i = w.i;
        let t = w.toks[i];
        w.i += 1;
        if let Kind::Punct(p) = t.kind {
            w.bracket(p);
            continue;
        }
        if t.kind != Kind::Ident || !w.at_top_level() {
            continue;
        }
        match w.text(t) {
            // A declaration starts its line: a `func` later on it is a function type.
            "func" if i == 0 || w.toks[i - 1].line < t.line => go_func(w, i),
            "type" if w.punct(i + 1) == Some(b'(') => {
                let end = w.skip_balanced(i + 1, b'(', b')');
                let mut depth = 0usize;
                for k in i + 2..end.saturating_sub(1) {
                    let tk = w.toks[k];
                    match tk.kind {
                        Kind::Punct(b'(' | b'[' | b'{') => depth += 1,
                        Kind::Punct(b')' | b']' | b'}') => depth = depth.saturating_sub(1),
                        Kind::Ident if depth == 0 && w.toks[k - 1].line < tk.line => {
                            w.emit(w.text(tk).to_owned(), k, SymbolKind::Type);
                        }
                        _ => {}
                    }
                }
                w.i = end;
            }
            "type" => {
                if let Some(name) = w.word(i + 1) {
                    w.emit(name.to_owned(), i + 1, SymbolKind::Type);
                }
            }
            _ => {}
        }
    }
}

fn go_func(w: &mut Walk<'_>, at: usize) {
    if let Some(name) = w.word(at + 1) {
        w.emit(name.to_owned(), at + 1, SymbolKind::Function);
        return;
    }
    if w.punct(at + 1) != Some(b'(') {
        return;
    }
    let end = w.skip_balanced(at + 1, b'(', b')');
    let mut recv = None;
    let mut depth = 0usize;
    for k in at + 2..end.saturating_sub(1) {
        match w.toks[k].kind {
            Kind::Punct(b'[') => depth += 1,
            Kind::Punct(b']') => depth = depth.saturating_sub(1),
            Kind::Ident if depth == 0 => recv = Some(w.text(w.toks[k])),
            _ => {}
        }
    }
    if let (Some(recv), Some(name)) = (recv, w.word(end)) {
        w.emit(format!("{recv}.{name}"), end, SymbolKind::Method);
    }
}
