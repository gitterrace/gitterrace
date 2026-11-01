//! Syntax highlighting: one syntax set, one scope-to-class map, one language detector.
//!
//! Output is HTML with `s-*` classes only (the stylesheets colour them), never inline styles.
//! The set is bat's (via `two-face`), loaded once on first use.

use std::collections::HashMap;
use std::sync::OnceLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

/// Larger inputs are shown plain: highlighting must never dominate a request.
pub(crate) const HIGHLIGHT_LIMIT: usize = 256 * 1024;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Colour categories the stylesheet knows, in stylesheet order: a later one wins within one scope.
const CATEGORIES: [(&str, &str); 13] = [
    ("comment", "s-comment"),
    ("keyword", "s-keyword"),
    ("storage", "s-keyword"),
    ("string", "s-string"),
    ("constant", "s-constant"),
    ("support", "s-constant"),
    ("entity", "s-entity"),
    ("variable", "s-entity"),
    ("tag", "s-tag"),
    ("numeric", "s-numeric"),
    ("invalid", "s-invalid"),
    ("punctuation", "s-punctuation"),
    ("parameter", "s-parameter"),
];

/// The colour class of one scope: among its atoms the stylesheet colours, the one whose rule comes last.
fn scope_class(scope: Scope) -> Option<&'static str> {
    let name = scope.build_string();
    name.split('.')
        .filter_map(|atom| CATEGORIES.iter().rposition(|(a, _)| *a == atom))
        .max()
        .map(|i| CATEGORIES[i].1)
}

/// Fence tokens and names people write that are not a syntax's own token.
fn alias(token: &str) -> &str {
    match token {
        "sh" | "shell" | "zsh" | "console" | "shell-session" | "bash" | "ksh" | "dash" => "sh",
        "yml" => "yaml",
        "rs" => "rs",
        "py" | "python3" | "python" => "py",
        "kt" | "kotlin" | "kts" => "kt",
        "ts" | "typescript" => "ts",
        "tsx" => "tsx",
        "js" | "javascript" | "node" | "mjs" | "cjs" => "js",
        "jsx" => "jsx",
        "rb" | "ruby" => "rb",
        "pl" | "perl" => "pl",
        "golang" => "go",
        "docker" | "dockerfile" => "Dockerfile",
        "make" | "makefile" => "Makefile",
        "ps1" | "powershell" | "pwsh" => "ps1",
        "objc" | "objective-c" => "m",
        "c++" | "cxx" => "cpp",
        "csharp" | "c#" => "cs",
        "tf" | "terraform" | "hcl" => "tf",
        "md" | "markdown" => "md",
        "jsonc" | "json5" => "json",
        "html" | "htm" | "xhtml" => "html",
        other => other,
    }
}

/// The syntax for a markdown fence token.
pub(crate) fn syntax_for_token(token: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let token = token.trim();
    set.find_syntax_by_token(alias(token))
        .or_else(|| set.find_syntax_by_token(&token.to_ascii_lowercase()))
}

/// The syntax for a shebang line such as `#!/usr/bin/env -S python3 -u`.
fn syntax_for_shebang(first: &str) -> Option<&'static SyntaxReference> {
    let rest = first.strip_prefix("#!")?;
    let mut words = rest.split_whitespace();
    let mut prog = words.next()?.rsplit('/').next()?;
    if prog == "env" {
        prog = words.find(|w| !w.starts_with('-') && !w.contains('='))?;
    }
    let name = prog.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    syntax_for_token(name).or_else(|| syntax_for_token(prog))
}

/// The syntax for a file: by name, then extension, then shebang, then the set's first-line rules.
pub(crate) fn syntax_for_file(path: &str, first_line: &str) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = path.rsplit('/').next().unwrap_or(path);
    let by_name = match name {
        "Justfile" | "justfile" | ".justfile" => set.find_syntax_by_token("Makefile"),
        "CMakeLists.txt" => set.find_syntax_by_token("CMake"),
        "Cargo.lock" | "uv.lock" | "poetry.lock" | "Pipfile.lock" => {
            set.find_syntax_by_token("toml")
        }
        "go.mod" | "go.sum" => set.find_syntax_by_token("go"),
        "Podfile" | "Gemfile" | "Rakefile" | "Fastfile" | "Brewfile" => {
            set.find_syntax_by_token("rb")
        }
        "Package.swift" => set.find_syntax_by_token("swift"),
        ".bashrc" | ".zshrc" | ".bash_profile" | ".zprofile" | ".profile" | ".zshenv" => {
            set.find_syntax_by_token("sh")
        }
        _ if name.starts_with("Dockerfile") || name.starts_with("Containerfile") => {
            set.find_syntax_by_token("Dockerfile")
        }
        _ => None,
    };
    by_name
        .or_else(|| set.find_syntax_by_extension(name))
        .or_else(|| {
            let ext = name.rsplit_once('.')?.1;
            set.find_syntax_by_extension(ext)
                .or_else(|| set.find_syntax_by_extension(&ext.to_ascii_lowercase()))
                .or_else(|| set.find_syntax_by_token(alias(ext)))
        })
        .or_else(|| syntax_for_shebang(first_line))
        .or_else(|| set.find_syntax_by_first_line(first_line))
        .filter(|s| s.name != "Plain Text")
}

/// Highlights line by line, keeping parse state across calls.
pub(crate) struct Highlighter {
    state: ParseState,
    stack: ScopeStack,
    cache: HashMap<Scope, Option<&'static str>>,
}

impl Highlighter {
    pub(crate) fn new(syntax: &'static SyntaxReference) -> Self {
        Self {
            state: ParseState::new(syntax),
            stack: ScopeStack::new(),
            cache: HashMap::new(),
        }
    }

    fn class(&mut self) -> Option<&'static str> {
        let cache = &mut self.cache;
        self.stack
            .as_slice()
            .iter()
            .rev()
            .find_map(|s| *cache.entry(*s).or_insert_with(|| scope_class(*s)))
    }

    /// The HTML of one line (no trailing newline in or out), spans closed at the end of it.
    /// On a parser error the line comes back escaped and plain.
    pub(crate) fn line(&mut self, text: &str) -> String {
        let mut buf = String::with_capacity(text.len() + 1);
        buf.push_str(text);
        buf.push('\n');
        let Ok(ops) = self.state.parse_line(&buf, syntaxes()) else {
            return crate::fmt::esc_text(text);
        };
        let body = &buf[..text.len()];
        let mut out = String::with_capacity(text.len() * 2);
        let mut prev = 0;
        let mut merged: Vec<(Option<&'static str>, usize, usize)> = Vec::new();
        for (i, op) in ops {
            let to = i.min(body.len());
            if to > prev {
                let class = self.class();
                match merged.last_mut() {
                    Some(last) if last.0 == class => last.2 = to,
                    _ => merged.push((class, prev, to)),
                }
                prev = to;
            }
            if self.stack.apply(&op).is_err() {
                return crate::fmt::esc_text(text);
            }
        }
        if body.len() > prev {
            let class = self.class();
            match merged.last_mut() {
                Some(last) if last.0 == class => last.2 = body.len(),
                _ => merged.push((class, prev, body.len())),
            }
        }
        for (class, a, b) in merged {
            if let Some(c) = class {
                out.push_str("<span class=\"");
                out.push_str(c);
                out.push_str("\">");
            }
            out.push_str(&crate::fmt::esc_text(&body[a..b]));
            if class.is_some() {
                out.push_str("</span>");
            }
        }
        out
    }
}

/// A highlighter for a file, or `None` when its language is unknown.
pub(crate) fn for_file(path: &str, first_line: &str) -> Option<Highlighter> {
    syntax_for_file(path, first_line).map(Highlighter::new)
}

/// Highlighted HTML of a whole block for a fence token; `None` when unknown or over the limit.
pub(crate) fn block(code: &str, token: &str) -> Option<String> {
    if code.len() > HIGHLIGHT_LIMIT {
        return None;
    }
    let mut h = Highlighter::new(syntax_for_token(token)?);
    let mut out = String::with_capacity(code.len() * 2);
    for line in code.split_inclusive('\n') {
        let (body, nl) = match line.strip_suffix('\n') {
            Some(b) => (b.strip_suffix('\r').unwrap_or(b), "\n"),
            None => (line, ""),
        };
        out.push_str(&h.line(body));
        out.push_str(nl);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (file name, source) that must produce a keyword or string span.
    const CASES: [(&str, &str); 17] = [
        ("a.py", "import os\nx = \"hi\"\n"),
        ("a.sh", "if [ -f x ]; then echo \"hi\"; fi\n"),
        ("a.bash", "for i in 1 2; do echo \"$i\"; done\n"),
        ("run", "#!/usr/bin/env python3\nimport os\nprint(\"x\")\n"),
        ("run2", "#!/bin/bash\nif true; then echo \"x\"; fi\n"),
        ("a.toml", "[package]\nname = \"x\"\n"),
        ("Cargo.lock", "[[package]]\nname = \"x\"\n"),
        ("a.ts", "const x: string = \"a\";\nexport function f() {}\n"),
        (
            "a.tsx",
            "const x = <div className=\"a\">hi</div>;\nexport default x;\n",
        ),
        ("a.swift", "import Foundation\nlet x = \"a\"\nfunc f() {}\n"),
        ("a.kt", "package a\nfun main() { val x = \"a\" }\n"),
        ("a.go", "package main\nfunc main() { x := \"a\" }\n"),
        ("a.rs", "fn main() { let x = \"a\"; }\n"),
        ("Dockerfile", "FROM rust:1\nRUN echo \"hi\"\n"),
        ("a.yaml", "key: \"v\"\nlist:\n  - true\n"),
        ("a.json", "{\"a\": \"b\"}\n"),
        ("a.sql", "SELECT \"a\" FROM t WHERE x = 'b';\n"),
    ];

    fn render(path: &str, src: &str) -> Option<String> {
        let first = src.lines().next().unwrap_or("");
        let mut h = for_file(path, first)?;
        Some(
            src.lines()
                .map(|l| h.line(l))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    #[test]
    fn common_languages_get_keyword_or_string_spans() {
        for (path, src) in CASES {
            let out = render(path, src).unwrap_or_else(|| panic!("{path}: no syntax"));
            assert!(
                out.contains("s-keyword") || out.contains("s-string"),
                "{path}: {out}"
            );
        }
        let md = block("# T\n\n```rust\nfn x() {}\n```\n", "md").expect("md");
        assert!(md.contains("s-"), "{md}");
    }

    #[test]
    fn fence_aliases_resolve() {
        for t in [
            "ts", "tsx", "sh", "zsh", "shell", "console", "yml", "rs", "py", "kt", "swift", "toml",
        ] {
            assert!(syntax_for_token(t).is_some(), "{t}");
        }
    }

    #[test]
    fn highlighted_text_equals_raw() {
        let src = "fn main() {\n    let s = \"a<b>&\"; // c\n}\n";
        let out = render("a.rs", src).unwrap();
        let mut text = String::new();
        let mut in_tag = false;
        for c in out.chars() {
            match c {
                '<' => in_tag = true,
                '>' if in_tag => in_tag = false,
                _ if !in_tag => text.push(c),
                _ => {}
            }
        }
        let text = text
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&");
        assert_eq!(text, src.trim_end_matches('\n'));
    }

    /// `HL_FILE=path cargo test --release -- --ignored hl_timing --nocapture`
    #[test]
    #[ignore = "manual timing"]
    fn hl_timing() {
        let path = std::env::var("HL_FILE").expect("HL_FILE");
        let text = std::fs::read_to_string(&path).expect("read");
        for run in 0..4 {
            let t = std::time::Instant::now();
            let (h, _) = crate::views::code_html(&path, &text);
            eprintln!(
                "run {run}: {} KB -> {} KB in {:?}",
                text.len() / 1024,
                h.len() / 1024,
                t.elapsed()
            );
        }
    }
}
