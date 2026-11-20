//! TypeScript and JavaScript declarations from the shared token walk.

use crate::symbols::SymbolKind;
use crate::symbols::lex::Kind;
use crate::symbols::walk::Walk;

/// Words that start a statement: a declarator list without a `;` ends before one.
const STATEMENTS: [&str; 8] =
    ["const", "let", "var", "function", "class", "export", "if", "return"];

/// TypeScript and JavaScript member modifiers, which may stand before a method name.
const MODIFIERS: [&str; 10] = [
    "static",
    "async",
    "get",
    "set",
    "public",
    "private",
    "protected",
    "readonly",
    "abstract",
    "override",
];

/// TypeScript and JavaScript: functions, classes, interfaces, types, enums, namespaces,
/// class methods and top-level `const f = (...) =>` or `= function`.
pub(crate) fn script(w: &mut Walk<'_>) {
    while w.i < w.toks.len() {
        let i = w.i;
        let t = w.toks[i];
        w.i += 1;
        if !w.at_item_level() {
            if let Kind::Punct(p) = t.kind {
                w.bracket(p);
            }
            continue;
        }
        if w.in_class_body() {
            class_member(w, i);
        }
        if let Kind::Punct(p) = t.kind {
            w.bracket(p);
            continue;
        }
        if w.in_class_body() {
            continue;
        }
        // A declaration keyword used as a property name (`x.type`, `{ class: 1 }`) is not one.
        if matches!(w.punct(i.wrapping_sub(1)), Some(b'.')) || w.punct(i + 1) == Some(b':') {
            continue;
        }
        match w.text(t) {
            "function" if !expression_position(w, i) => {
                let k = if w.punct(i + 1) == Some(b'*') { i + 2 } else { i + 1 };
                if let Some(name) = w.word(k) {
                    w.emit(name.to_owned(), k, SymbolKind::Function);
                }
            }
            "class" => {
                let declared = !expression_position(w, i);
                if declared
                    && let Some(name) =
                        w.word(i + 1).filter(|n| *n != "extends" && *n != "implements")
                {
                    w.emit(name.to_owned(), i + 1, SymbolKind::Class);
                }
                // A class expression is listed only as a declarator's value (`var A = class`).
                if declared || declarator_value(w, i) {
                    w.pending = Some(true);
                }
                // The body is the first `{` after the heritage clause, past any generics.
                skip_to_body(w, i + 1);
            }
            "interface" if w.word(i + 1).is_some() => {
                w.emit(w.word(i + 1).unwrap_or_default().to_owned(), i + 1, SymbolKind::Interface);
            }
            "enum" if w.word(i + 1).is_some() => {
                w.emit(w.word(i + 1).unwrap_or_default().to_owned(), i + 1, SymbolKind::Enum);
            }
            "type" if w.word(i + 1).is_some() && matches!(w.punct(i + 2), Some(b'=' | b'<')) => {
                w.emit(w.word(i + 1).unwrap_or_default().to_owned(), i + 1, SymbolKind::Type);
            }
            "namespace" | "module"
                if w.tok(i + 1).is_some_and(|n| !matches!(n.kind, Kind::Punct(_)))
                    && w.punct(i + 2) == Some(b'{') =>
            {
                let name = w.text(w.toks[i + 1]).to_owned();
                let kind =
                    if w.text(t) == "module" { SymbolKind::Module } else { SymbolKind::Namespace };
                w.emit(name, i + 1, kind);
                w.pending = Some(false);
            }
            "const" | "let" | "var" => declarators(w, i),
            "global" if w.word(i.wrapping_sub(1)) == Some("declare") => {
                w.emit("global".to_owned(), i, SymbolKind::Namespace);
                w.pending = Some(false);
            }
            _ => {}
        }
    }
}

/// Whether the keyword at `k` stands where an expression goes (`x = function f`, `(class {})`):
/// a function or class expression, not a declaration.
fn expression_position(w: &Walk<'_>, k: usize) -> bool {
    let Some(p) = k.checked_sub(1).and_then(|p| w.tok(p)) else {
        return false;
    };
    match p.kind {
        Kind::Punct(c) => {
            matches!(c, b'=' | b'(' | b',' | b':' | b'?' | b'[' | b'!' | b'&' | b'|' | b'>')
        }
        Kind::Ident => matches!(w.text(p), "return" | "new" | "yield" | "await"),
        Kind::Lit => false,
    }
}

/// Whether the token at `k` is a declarator's whole value: `const A = class`, `, B = class`.
fn declarator_value(w: &Walk<'_>, k: usize) -> bool {
    w.punct(k.wrapping_sub(1)) == Some(b'=')
        && w.word(k.wrapping_sub(2)).is_some()
        && (matches!(w.word(k.wrapping_sub(3)), Some("const" | "let" | "var"))
            || w.punct(k.wrapping_sub(3)) == Some(b','))
}

/// Moves the walk to the class body's `{`, so a `{` in the heritage clause's type arguments
/// does not take the class body's place.
fn skip_to_body(w: &mut Walk<'_>, from: usize) {
    let mut k = from;
    while let Some(t) = w.tok(k) {
        match t.kind {
            Kind::Punct(b'{') => break,
            Kind::Punct(b'<' | b'(') => {
                k = skip_nested(w, k);
                continue;
            }
            _ => {}
        }
        k += 1;
    }
    w.i = k;
}

/// The index past the bracketed run at `k`, counting every bracket kind, so type arguments
/// may hold object types (`Base<{ a: 1 }>`). The `>` of an arrow `=>` closes nothing.
fn skip_nested(w: &Walk<'_>, k: usize) -> usize {
    let mut depth = 0usize;
    let mut j = k;
    while let Some(t) = w.tok(j) {
        j += 1;
        match t.kind {
            Kind::Punct(b'<' | b'(' | b'[' | b'{') => depth += 1,
            Kind::Punct(b'>') if w.punct(j - 2) == Some(b'=') => {}
            Kind::Punct(b'>' | b')' | b']' | b'}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return j;
                }
            }
            _ => {}
        }
    }
    j
}

/// The `=` of a declaration whose name ends at `k - 1`, past a type annotation. `None` when
/// the declaration has no initializer.
fn initializer(w: &Walk<'_>, k: usize) -> Option<usize> {
    match w.punct(k)? {
        b'=' => Some(k),
        b':' => {
            let mut depth = 0usize;
            let mut j = k + 1;
            while let Some(t) = w.tok(j) {
                match t.kind {
                    Kind::Punct(b'(' | b'[' | b'{' | b'<') => depth += 1,
                    Kind::Punct(b')' | b']' | b'}') => depth = depth.checked_sub(1)?,
                    Kind::Punct(b'>') if w.punct(j - 1) != Some(b'=') => {
                        depth = depth.saturating_sub(1);
                    }
                    Kind::Punct(b';' | b',') if depth == 0 => return None,
                    // No `=` before the next statement: a declaration without a value.
                    Kind::Ident if depth == 0 && STATEMENTS.contains(&w.text(t)) => return None,
                    Kind::Punct(b'=') if depth == 0 && w.punct(j + 1) != Some(b'>') => {
                        return Some(j);
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => None,
    }
}

/// Whether the expression at `k` is a function: `function`, `async`, `x =>` or `(...) =>`.
fn fn_value(w: &Walk<'_>, k: usize) -> bool {
    match (w.word(k), w.punct(k)) {
        (Some("function" | "async"), _) => true,
        (Some(_), _) => w.punct(k + 1) == Some(b'=') && w.punct(k + 2) == Some(b'>'),
        (None, Some(b'(')) => {
            let end = w.skip_balanced(k, b'(', b')');
            matches!((w.punct(end), w.punct(end + 1)), (Some(b'='), Some(b'>')) | (Some(b':'), _))
        }
        // Generic arrow `<T>(x: T) => x`, not a type assertion `<const> {}`.
        (None, Some(b'<')) => {
            let end = skip_nested(w, k);
            w.punct(end) == Some(b'(') && fn_value(w, end)
        }
        _ => false,
    }
}

/// Whether the token at `k`, at class-body level, starts a member: after the body's `{`, a
/// member's `}` or `;`, a modifier, or a decorator or field on an earlier line.
fn member_start(w: &Walk<'_>, k: usize) -> bool {
    let t = w.toks[k];
    let Some(p) = k.checked_sub(1).and_then(|p| w.tok(p)) else {
        return true;
    };
    match p.kind {
        Kind::Punct(b'{' | b'}' | b';' | b'*') => true,
        Kind::Ident if MODIFIERS.contains(&w.text(p)) => true,
        Kind::Ident | Kind::Lit | Kind::Punct(b')' | b']') => p.line < t.line,
        Kind::Punct(_) => false,
    }
}

/// A class member starting at `k`: a method when its name is followed by `(` or `<`. A
/// computed name (`[Symbol.dispose]`) is listed as written.
fn class_member(w: &mut Walk<'_>, k: usize) {
    let t = w.toks[k];
    if !matches!(t.kind, Kind::Ident | Kind::Punct(b'[')) || !member_start(w, k) {
        return;
    }
    let (name, mut next) = if t.kind == Kind::Punct(b'[') {
        let end = skip_nested(w, k);
        let last = w.toks[end - 1];
        (w.src[t.start as usize..last.end as usize].to_owned(), end)
    } else {
        let name = w.text(t);
        // A modifier followed by a name is not the member's name.
        if MODIFIERS.contains(&name)
            && w.tok(k + 1).is_some_and(|n| n.kind == Kind::Ident || n.kind == Kind::Punct(b'['))
        {
            return;
        }
        (name.to_owned(), k + 1)
    };
    if matches!(w.punct(next), Some(b'?' | b'!')) {
        next += 1;
    }
    if matches!(w.punct(next), Some(b'(' | b'<')) {
        w.emit(name, k, SymbolKind::Method);
    }
}

/// The declarators of a `const`, `let` or `var` at `k` (the keyword): each name whose value
/// is a function or a class is listed. Looks ahead only; the walk still visits every token.
fn declarators(w: &mut Walk<'_>, keyword: usize) {
    let mut at = keyword + 1;
    loop {
        let Some(name) = w.word(at) else { return };
        if w.punct(at + 1) == Some(b',') {
            at += 2;
            continue;
        }
        let Some(eq) = initializer(w, at + 1) else {
            return;
        };
        if w.word(eq + 1) == Some("class") {
            w.emit(name.to_owned(), at, SymbolKind::Class);
        } else if fn_value(w, eq + 1) {
            w.emit(name.to_owned(), at, SymbolKind::Function);
        }
        // To the next top-level `,`: past the value, which ends at `;` or a new statement.
        let mut depth = 0usize;
        at = eq + 1;
        loop {
            let Some(tok) = w.tok(at) else { return };
            match tok.kind {
                Kind::Punct(b'(' | b'[' | b'{') => depth += 1,
                Kind::Punct(b')' | b']' | b'}') => {
                    let Some(less) = depth.checked_sub(1) else {
                        return;
                    };
                    depth = less;
                }
                Kind::Punct(b';') if depth == 0 => return,
                Kind::Punct(b',') if depth == 0 => break,
                Kind::Ident
                    if depth == 0
                        && w.toks[at - 1].line < tok.line
                        && STATEMENTS.contains(&w.text(tok)) =>
                {
                    return;
                }
                _ => {}
            }
            at += 1;
        }
        at += 1;
    }
}
