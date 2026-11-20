//! The scanners against hand-checked sources: each case is a class of declaration a real
//! parser lists (or does not) that a naive keyword scan gets wrong.

use crate::symbols::{SymbolKind, outline, symbols};

/// `(line, name, depth)` of each symbol of `src` read as `path`.
fn listed(path: &str, src: &str) -> Vec<(u32, String, u8)> {
    symbols(path, src)
        .into_iter()
        .map(|s| (s.line, s.name, s.depth))
        .collect()
}

fn names(path: &str, src: &str) -> Vec<String> {
    listed(path, src).into_iter().map(|s| s.1).collect()
}

#[test]
fn rust_items_impls_and_methods_are_listed_with_their_lines() {
    let src = "\
//! fn not_a_doc_item() {}
pub struct Point { x: i32 }
enum Shape { Dot }
pub(crate) trait Area {
    fn area(&self) -> f64;
}
impl<T: Clone> Area for Wrapper<T> where T: Copy {
    fn area(&self) -> f64 { fn inner() {} 0.0 }
}
mod geo {
    pub fn far() {}
}
macro_rules! square { ($x:expr) => { $x * $x }; }
const S: &str = \"fn fake() {}\";
fn r#match() {}
";
    assert_eq!(
        listed("a.rs", src),
        [
            (2, "Point".into(), 0),
            (3, "Shape".into(), 0),
            (4, "Area".into(), 0),
            (5, "area".into(), 1),
            (7, "Area for Wrapper<T>".into(), 0),
            (8, "area".into(), 1),
            (10, "geo".into(), 0),
            (11, "far".into(), 1),
            (13, "square".into(), 0),
            (15, "match".into(), 0),
        ]
    );
}

#[test]
fn rust_impl_in_a_return_type_or_an_argument_is_not_an_impl_block() {
    let src = "fn it() -> impl Iterator<Item = u8> { [].into_iter() }\n\
               fn take(x: impl Fn()) {}\n";
    assert_eq!(names("a.rs", src), ["it", "take"]);
}

#[test]
fn rust_impl_headers_with_brackets_still_open_a_container() {
    // `[u8]`, `[T; N]` and `Fn()` in the header must not cancel the body that follows.
    let src = "\
impl AsRef<[u8]> for B { fn as_ref(&self) -> &[u8] { &[] } }
impl<T, const N: usize> S for [T; N] { fn s() {} }
impl<F> W<F> where F: Fn() -> u8 { fn w() {} }
";
    let got = listed("a.rs", src);
    let methods: Vec<_> = got
        .iter()
        .filter(|s| s.2 == 1)
        .map(|s| s.1.as_str())
        .collect();
    assert_eq!(methods, ["as_ref", "s", "w"]);
}

#[test]
fn rust_raw_strings_chars_and_lifetimes_hide_nothing() {
    let src = "\
const A: &str = r#\"\" fn in_raw() {} \"#;
const B: char = '{';
fn life<'a>(x: &'a str) -> &'a str { x }
const C: &[u8] = b\"fn in_bytes() {}\";
fn after() {}
";
    assert_eq!(names("a.rs", src), ["life", "after"]);
}

#[test]
fn go_functions_methods_and_types_are_listed() {
    let src = "\
package p
type (
\tA struct{ f func() int }
\tB = int
)
type C interface{ M() }
func F() {}
func (c *C) M() {}
func (g G[T]) N() {}
func (fd *netFD) addr() func(int) error { return nil }
var s = `func Fake() {}`
";
    assert_eq!(
        names("a.go", src),
        ["A", "B", "C", "F", "C.M", "G.N", "netFD.addr"]
    );
}

#[test]
fn python_lists_defs_and_classes_but_not_nested_functions_or_strings() {
    let src = "\
import os
class A(Base):
    \"\"\"def not_a_def(): pass\"\"\"
    def m(self):
        def helper(): pass
        return 1
    async def am(self): pass
def f(x,
      y): pass
x = '''
def in_string(): pass
'''
if os.name == \"nt\":
    def windows(): pass
try:
    from fast import Thing
except ImportError:
    class Thing:
        def __init__(self): pass
";
    assert_eq!(
        listed("a.py", src),
        [
            (2, "A".into(), 0),
            (4, "m".into(), 1),
            (7, "am".into(), 1),
            (8, "f".into(), 0),
            (14, "windows".into(), 0),
            (18, "Thing".into(), 0),
            (19, "__init__".into(), 1),
        ]
    );
}

#[test]
fn typescript_declarations_and_class_members_are_listed() {
    let src = "\
export interface Opts { a: number }
export type Id<T> = T;
enum E { A }
export abstract class Box<T extends { k: 1 }> extends Base<T> implements I {
  #secret = 1;
  private readonly x: number = 2;
  static async make(): Promise<Box<number>> { return null!; }
  get size(): number { return 1; }
  [Symbol.dispose](): void {}
  constructor(private y: number) { super(); }
  abstract get id(): string;
  run = () => { function inner() {} };
}
export function top<T>(x: T): T { return x; }
const arrow = async (a: number): Promise<void> => {};
const typed: () => number = () => 1;
const notFn = compute(1);
declare global { interface Window { x: 1 } }
";
    assert_eq!(
        names("a.ts", src),
        [
            "Opts",
            "Id",
            "E",
            "Box",
            "make",
            "size",
            "[Symbol.dispose]",
            "constructor",
            "id",
            "top",
            "arrow",
            "typed",
            "global",
            "Window",
        ]
    );
}

#[test]
fn script_regex_and_template_literals_hide_nothing() {
    let src = "\
const re = /function fake() {}/g;
const t = `class Fake { ${ (() => { return 1; })() } }`;
function real() { return 6 / 2 / 1; }
";
    assert_eq!(names("a.js", src), ["real"]);
}

#[test]
fn javascript_class_expressions_and_declarator_lists_are_listed() {
    let src = "\
var a = 1, B = class extends A {
  m() {}
}, c = (x) => x;
const mixin = (S) => class Mixin extends S { n() {} };
module.exports.f = function f() {};
const TYPE = <const> { ALL: 0 };
";
    assert_eq!(names("a.js", src), ["B", "m", "c", "mixin"]);
}

#[test]
fn files_in_other_languages_have_no_symbols() {
    assert_eq!(symbols("a.c", "int main() { return 0; }\n").len(), 0);
    assert_eq!(symbols("notes.txt", "fn main() {}\n").len(), 0);
}

#[test]
fn the_outline_links_each_symbol_to_its_line_and_stops_at_the_shown_lines() {
    let src = "struct A;\nimpl A {\n    fn b() {}\n}\nfn c() {}\n";
    let items = outline("a.rs", src, 4, Vec::new());
    let got: Vec<_> = items
        .iter()
        .map(|i| (i.level, i.id.as_str(), i.text.as_str(), i.chrome))
        .collect();
    assert_eq!(
        got,
        [
            (2, "L1", "A", true),
            (2, "L2", "A", true),
            (3, "L3", "b", true)
        ]
    );
}

/// `(name, kind)` of each symbol of `src` read as `path`.
fn kinds(path: &str, src: &str) -> Vec<(String, Option<SymbolKind>)> {
    symbols(path, src)
        .into_iter()
        .map(|s| (s.name, s.kind))
        .collect()
}

#[test]
fn rust_symbols_carry_the_kind_their_keyword_names() {
    use SymbolKind::*;
    let src = "struct S;\nenum E {}\nunion U { a: u8 }\ntrait T { fn m(&self); }\n\
               type A = u8;\nmod m {}\nimpl T for S { fn m(&self) {} }\n\
               macro_rules! mac { () => {}; }\nfn f() {}\n";
    assert_eq!(
        kinds("a.rs", src),
        [
            ("S".into(), Some(Struct)),
            ("E".into(), Some(Enum)),
            ("U".into(), Some(Union)),
            ("T".into(), Some(Trait)),
            ("m".into(), Some(Method)),
            ("A".into(), Some(Type)),
            ("m".into(), Some(Module)),
            ("T for S".into(), Some(Impl)),
            ("m".into(), Some(Method)),
            ("mac".into(), Some(Macro)),
            ("f".into(), Some(Function)),
        ]
    );
}

#[test]
fn go_symbols_carry_their_kind() {
    use SymbolKind::*;
    let src = "package p\ntype A int\ntype (\n\tB int\n)\nfunc F() {}\nfunc (a A) M() {}\n";
    assert_eq!(
        kinds("a.go", src),
        [
            ("A".into(), Some(Type)),
            ("B".into(), Some(Type)),
            ("F".into(), Some(Function)),
            ("A.M".into(), Some(Method)),
        ]
    );
}

#[test]
fn python_symbols_carry_their_kind() {
    use SymbolKind::*;
    let src = "class C:\n    def m(self):\n        pass\n\nasync def f():\n    pass\n";
    assert_eq!(
        kinds("a.py", src),
        [
            ("C".into(), Some(Class)),
            ("m".into(), Some(Method)),
            ("f".into(), Some(Function)),
        ]
    );
}

#[test]
fn script_symbols_carry_their_kind() {
    use SymbolKind::*;
    let src = "function f() {}\nclass C { m() {} }\ninterface I {}\nenum E {}\n\
               type T = number;\nnamespace N {}\ndeclare module M {}\n\
               const g = () => 1;\nconst K = class {};\n";
    assert_eq!(
        kinds("a.ts", src),
        [
            ("f".into(), Some(Function)),
            ("C".into(), Some(Class)),
            ("m".into(), Some(Method)),
            ("I".into(), Some(Interface)),
            ("E".into(), Some(Enum)),
            ("T".into(), Some(Type)),
            ("N".into(), Some(Namespace)),
            ("M".into(), Some(Module)),
            ("g".into(), Some(Function)),
            ("K".into(), Some(Class)),
        ]
    );
}

#[test]
fn the_outline_shows_each_kind_as_a_label_and_speaks_the_full_word() {
    let src = "struct A;\nimpl A {\n    fn b() {}\n}\nfn c() {}\n";
    let rail = crate::markdown::outline_rail(&outline("a.rs", src, 9, Vec::new()));
    assert!(rail.contains("class=\"kinds kw6\""), "{rail}");
    assert!(
        rail.contains(
            ">fn</span><span class=\"vh\">function </span><span class=\"oname\">c</span>"
        )
    );
    assert!(rail.contains(">impl</span><span class=\"vh\">implementation </span>"));
    let doc = [("a", "A"), ("b", "B"), ("c", "C")]
        .map(|(id, t)| crate::markdown::OutlineItem::heading(2, id.into(), t));
    assert!(!crate::markdown::outline_rail(&doc).contains("okind"));
}

/// Every grammar in the sample set names a function and a type in a small file, and the
/// highlight pass turns them into an outline: a language whose grammar marks declarations
/// needs no code of its own. The fixtures are under `tests/fixtures/outline`.
#[test]
fn every_sample_grammar_outlines_a_function_and_a_type() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/outline");
    let mut seen = 0;
    for e in std::fs::read_dir(&dir).expect("fixtures") {
        let p = e.expect("entry").path();
        let text = std::fs::read_to_string(&p).expect("fixture");
        let name = p.file_name().and_then(|n| n.to_str()).expect("name");
        let r = crate::views::Render::test(0);
        let named = crate::views::code(&r, name, &text).symbols;
        let items = outline(name, &text, usize::MAX, named);
        let kinds: Vec<_> = items.iter().filter_map(|i| i.kind).collect();
        let func = kinds
            .iter()
            .any(|k| matches!(k, SymbolKind::Function | SymbolKind::Method));
        assert!(func, "{name}: no function in {kinds:?}");
        // Lua, shell and Haskell grammars name no type declarations.
        let typed = kinds
            .iter()
            .any(|k| !matches!(k, SymbolKind::Function | SymbolKind::Method));
        let untyped = ["a.lua", "a.sh", "a.hs"].contains(&name);
        assert!(typed || untyped, "{name}: no type in {kinds:?}");
        seen += 1;
    }
    assert!(seen >= 10, "only {seen} sample languages");
}

/// Collecting symbols rides the highlight pass: on a 5000-line file the scope names it builds
/// stay a small share of the scope pushes the highlighter makes anyway. Measured on real
/// 5000-line Rust, Java and C files the collection adds 0.3 to 3% to highlighting; the share
/// counted here is the part that grows with the file, held under 15%.
#[test]
fn symbol_collection_is_a_small_share_of_the_highlight_pass() {
    let unit = "/** A doc. */\npublic class Shop {\n    private int n = 0;\n\
                public int total(int a) {\n        if (a > 0) { return a + n; }\n\
                return \"class Decoy\".length();\n    }\n    interface Item { int price(); }\n}\n";
    let text = unit.repeat(5000 / unit.lines().count());
    let mut h = crate::highlight::for_file("big.java", "")
        .expect("java")
        .with_symbols();
    text.lines().for_each(|l| drop(h.line(l)));
    let (pushes, work) = h.symbol_work();
    assert!(h.take_symbols().len() >= 1000, "the outline is real");
    assert!(
        work * 100 < pushes * 15,
        "{work} names built for {pushes} pushes"
    );
}
