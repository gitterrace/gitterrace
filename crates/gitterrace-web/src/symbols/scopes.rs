//! The outline a grammar gives for free: bat's grammars name each declaration with an
//! `entity.name.*` scope inside `meta.function`, `meta.class` and the like, so the highlight
//! pass that already walks those scopes records the symbols as it goes, with no second parse.
//!
//! Only the scope pushes are looked at, and a scope's kind is read once per distinct scope, so
//! collecting costs a map lookup per push on top of the highlighting.

use std::collections::HashMap;

use syntect::parsing::{Scope, ScopeStack, ScopeStackOp};

use crate::symbols::{Symbol, SymbolKind};

/// Meta scopes whose body holds members: a declaration inside one is nested.
const CONTAINERS: [&str; 12] = [
    "class",
    "struct",
    "interface",
    "trait",
    "impl",
    "namespace",
    "module",
    "enum",
    "union",
    "object",
    "protocol",
    "extension",
];

/// What an `entity.name.*` scope declares, from the first atom after `name` the outline
/// knows: `entity.name.type.class.kotlin` is a class, `entity.name.function.preprocessor.c`
/// a macro. `None` for names that are not declarations (tags, sections, labels, constants)
/// and for packages, which every file of a package repeats.
fn kind_of(name: &str) -> Option<SymbolKind> {
    let rest = name.strip_prefix("entity.name.")?;
    let atoms: Vec<&str> = rest.split('.').collect();
    if atoms.iter().any(|a| matches!(*a, "package" | "header")) {
        return None;
    }
    if atoms.first() == Some(&"function") && atoms.contains(&"preprocessor") {
        return Some(SymbolKind::Macro);
    }
    atoms
        .iter()
        .find_map(|a| match *a {
            "function" | "method" => Some(SymbolKind::Function),
            "class" | "object" => Some(SymbolKind::Class),
            "struct" => Some(SymbolKind::Struct),
            "enum" => Some(SymbolKind::Enum),
            "union" => Some(SymbolKind::Union),
            "trait" | "protocol" => Some(SymbolKind::Trait),
            "interface" => Some(SymbolKind::Interface),
            "impl" => Some(SymbolKind::Impl),
            "namespace" => Some(SymbolKind::Namespace),
            "module" => Some(SymbolKind::Module),
            "macro" => Some(SymbolKind::Macro),
            _ => None,
        })
        .or_else(|| {
            atoms.iter().any(|a| matches!(*a, "type" | "typedef")).then_some(SymbolKind::Type)
        })
}

/// Whether a kind is itself a container, whose own meta scope is open around its name.
const fn contains(kind: SymbolKind) -> bool {
    !matches!(
        kind,
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Macro | SymbolKind::Type
    )
}

/// What one open scope says about the declarations inside it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Meta {
    /// Says nothing.
    Other,
    /// A package or import line: nothing in it is listed.
    Skip,
    /// A function's whole declaration (`meta.function.c`).
    Function,
    /// A container's whole declaration (`meta.class.java`).
    Container,
    /// A container's body (`meta.class.body.java`).
    Body,
}

fn meta_of(name: &str) -> Meta {
    let atoms: Vec<&str> = name.split('.').collect();
    if atoms.first() != Some(&"meta") || atoms.len() < 3 {
        return Meta::Other;
    }
    let (what, sub) = (atoms[1], atoms[2]);
    if what.starts_with("package") || what == "import" {
        Meta::Skip
    } else if matches!(what, "function" | "method") && atoms.len() == 3 {
        Meta::Function
    } else if CONTAINERS.contains(&what) && sub == "body" {
        Meta::Body
    } else if CONTAINERS.contains(&what) && atoms.len() == 3 {
        Meta::Container
    } else {
        Meta::Other
    }
}

/// Where a declaration sits, from what the scopes open around it say: `None` to skip it
/// (inside a function body, or in a package line), else its depth (1 in a container body).
fn place(metas: &[Meta], kind: SymbolKind) -> Option<u8> {
    let mut functions = 0_u32;
    let mut containers = 0_u8;
    let mut body = false;
    for m in metas {
        match m {
            Meta::Skip => return None,
            Meta::Function => functions += 1,
            Meta::Container => containers += 1,
            Meta::Body => body = true,
            Meta::Other => {}
        }
    }
    let own = u8::from(contains(kind));
    let depth = u8::from(body || containers > own);
    // The function's own meta scope is open around its name: one more is a body.
    let limit = u32::from(kind == SymbolKind::Function);
    (functions <= limit).then_some(depth)
}

/// Records declarations as the highlighter applies each line's scope operations.
#[derive(Default)]
pub(crate) struct Collector {
    kinds: HashMap<Scope, Option<SymbolKind>>,
    metas: HashMap<Scope, Meta>,
    /// What the open scopes say, scratch reused per declaration.
    around: Vec<Meta>,
    /// The name being read: the stack depth below its scope, its start byte and its kind.
    open: Option<(usize, usize, SymbolKind, u8)>,
    line: u32,
    /// Where the last name ended on this line, so `Shop.Cart` pushed as two names is one.
    end: Option<usize>,
    pub(crate) out: Vec<Symbol>,
    /// The pushes seen, and the scope names built for them: the cost the perf guard counts.
    pub(crate) pushes: u64,
    pub(crate) work: u64,
}

impl Collector {
    /// Starts the next line.
    pub(crate) const fn next_line(&mut self) {
        self.line += 1;
        self.open = None;
        self.end = None;
    }

    /// Called before `op` at byte `at` is applied to `stack`.
    pub(crate) fn before(&mut self, stack: &ScopeStack, at: usize, op: &ScopeStackOp) {
        let ScopeStackOp::Push(scope) = op else {
            return;
        };
        self.pushes += 1;
        if self.open.is_some() {
            return;
        }
        let work = &mut self.work;
        let kind = *self.kinds.entry(*scope).or_insert_with(|| {
            *work += 1;
            kind_of(&scope.build_string())
        });
        let Some(kind) = kind else {
            return;
        };
        let below = stack.len();
        self.around.clear();
        let mut prev = None;
        for s in stack.as_slice() {
            // A grammar may push one meta scope twice (C's `meta.struct.c`): one container.
            if prev.replace(*s) == Some(*s) {
                continue;
            }
            let work = &mut self.work;
            let m = *self.metas.entry(*s).or_insert_with(|| {
                *work += 1;
                meta_of(&s.build_string())
            });
            self.around.push(m);
        }
        if let Some(depth) = place(&self.around, kind) {
            self.open = Some((below, at, kind, depth));
        }
    }

    /// Called after an op at byte `at`: a name ends when its scope pops.
    pub(crate) fn after(&mut self, stack: &ScopeStack, at: usize, body: &str) {
        if let Some((below, ..)) = self.open
            && stack.len() <= below
        {
            self.close(at, body);
        }
    }

    /// Ends the open name at byte `at` of the line `body`.
    pub(crate) fn close(&mut self, at: usize, body: &str) {
        let Some((_, from, kind, depth)) = self.open.take() else {
            return;
        };
        let at = at.min(body.len());
        let name = body.get(from..at).unwrap_or("").trim();
        if name.is_empty() {
            return;
        }
        let joined = self.end.is_some_and(|e| body.get(e..from) == Some("."));
        self.end = Some(at);
        if joined && let Some(last) = self.out.last_mut() {
            last.name.push('.');
            last.name.push_str(name);
            return;
        }
        let kind =
            if kind == SymbolKind::Function && depth > 0 { SymbolKind::Method } else { kind };
        self.out.push(Symbol { name: name.to_owned(), line: self.line, depth, kind: Some(kind) });
    }
}

#[cfg(test)]
mod tests {
    use super::kind_of;
    use crate::symbols::SymbolKind;

    #[test]
    fn scope_suffixes_map_to_kinds() {
        let cases = [
            ("entity.name.function.c", Some(SymbolKind::Function)),
            ("entity.name.type.class.kotlin", Some(SymbolKind::Class)),
            ("entity.name.function.preprocessor.c", Some(SymbolKind::Macro)),
            ("entity.name.namespace.header.scala", None),
            ("entity.name.package.kotlin", None),
            ("entity.name.tag.html", None),
            ("entity.name.section.markdown", None),
            ("entity.name.type.typedef.c", Some(SymbolKind::Type)),
        ];
        for (scope, want) in cases {
            assert_eq!(kind_of(scope), want, "{scope}");
        }
    }
}
