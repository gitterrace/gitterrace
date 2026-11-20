//! The file page of a source file in each outlined language, against
//! `tests/golden/symbols.txt`: the whole page, so the outline rail, the path bar menu and the
//! code they link into are pinned together. Kept apart from the matrix goldens: a new file in
//! the shared fixture would move every insights answer.

use std::collections::BTreeMap;
use std::path::Path;

use gitterrace_testkit::{Fixture, oracle};

/// One file per outlined language, each with a container and its members, a nested function
/// that is not listed and a decoy keyword in a string.
const FILES: [(&str, &str); 6] = [
    (
        "src/shapes.rs",
        "/// A shape.\npub trait Shape {\n    fn area(&self) -> f64;\n}\n\n\
         pub struct Square(f64);\n\nimpl Shape for Square {\n    fn area(&self) -> f64 {\n        \
         fn half(x: f64) -> f64 { x / 2.0 }\n        half(self.0) * 2.0\n    }\n}\n\n\
         pub fn label() -> &'static str { \"fn decoy() {}\" }\n",
    ),
    (
        "cmd/serve.go",
        "package main\n\ntype Server struct{ addr string }\n\n\
         func (s *Server) Start() error { return nil }\n\n\
         func main() {\n\tf := func() {}\n\tf()\n\t_ = \"func decoy() {}\"\n}\n",
    ),
    (
        "tools/report.py",
        "import sys\n\n\nclass Report:\n    \"\"\"def decoy(): pass\"\"\"\n\n    \
         def __init__(self, rows):\n        self.rows = rows\n\n    def total(self):\n        \
         def add(a, b):\n            return a + b\n        return sum(self.rows)\n\n\n\
         def main():\n    print(Report([1]).total())\n",
    ),
    (
        "web/store.ts",
        "export interface Item { id: number }\n\nexport class Store<T extends Item> {\n  \
         private items: T[] = [];\n  add(item: T): void { this.items.push(item); }\n  \
         get size(): number { return this.items.length; }\n}\n\n\
         export const empty = (): Store<Item> => new Store();\n\
         const decoy = `class Decoy {}`;\n",
    ),
    (
        "web/view.js",
        "export class View {\n  constructor(root) { this.root = root; }\n  \
         render() { return /function decoy\\(\\)/.test(\"\"); }\n}\n\n\
         export function mount(el) {\n  function inner() {}\n  return new View(el);\n}\n",
    ),
    ("src/two.rs", "fn one() {}\n\nfn two() {}\n"),
];

#[test]
fn source_pages_with_their_symbol_outline_match_the_oracle() {
    if !oracle::frozen("source_pages_with_their_symbol_outline_match_the_oracle") {
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let dir = tempfile::tempdir().expect("tmp");
    let answers = rt.block_on(async {
        let f = FILES
            .iter()
            .fold(Fixture::init(&dir.path().join("repo")), |f, (p, body)| f.write(p, body))
            .at("2024-03-01T10:00:00Z")
            .commit("feat: one file per outlined language");
        let router = gitterrace_web::app_router(f.path()).expect("router");
        // A page carries the repo id only once it is known: answer after ready, as a user would.
        oracle::ready(&router).await;
        let mut got = BTreeMap::new();
        for (p, _) in FILES {
            let uri = format!("/blob/HEAD/{p}");
            let (status, body) = gitterrace_testkit::page(&router, &uri).await;
            let body = oracle::normalize(&body).replace(&*dir.path().to_string_lossy(), "<TMP>");
            assert_eq!(status, 200, "{uri}");
            got.insert(format!("[symbols] {uri}"), body);
        }
        got
    });
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/symbols.txt");
    let dump = Path::new(env!("CARGO_TARGET_TMPDIR")).join("symbols.actual");
    oracle::check(&golden, &dump, &answers);
}
