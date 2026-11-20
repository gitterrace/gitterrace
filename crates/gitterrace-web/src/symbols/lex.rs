//! A tokenizer for the brace languages: identifiers and punctuation, with comments and string,
//! char and regex literals skipped, so a keyword inside one is never read as code.

/// One token: its kind, byte range in the source and 1-based line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Tok {
    pub(crate) kind: Kind,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Ident,
    Lit,
    Punct(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    Rust,
    Go,
    Script,
}

/// Words after which a `/` starts a regex rather than a division.
const REGEX_AFTER: [&str; 15] = [
    "return",
    "typeof",
    "instanceof",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "case",
    "do",
    "else",
    "yield",
    "await",
    "extends",
];

const fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80
}

const fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

struct Lexer<'a> {
    src: &'a [u8],
    at: usize,
    line: u32,
    dialect: Dialect,
    toks: Vec<Tok>,
    /// Open `{` per template literal nesting: a `}` at zero resumes the template text.
    templates: Vec<u32>,
}

impl Lexer<'_> {
    fn peek(&self, k: usize) -> u8 {
        self.src.get(self.at + k).copied().unwrap_or(0)
    }

    fn push(&mut self, kind: Kind, start: usize, line: u32) {
        self.toks.push(Tok {
            kind,
            start: u32::try_from(start).unwrap_or(u32::MAX),
            end: u32::try_from(self.at).unwrap_or(u32::MAX),
            line,
        });
    }

    /// Skips to just past the closing `close`, honouring backslash escapes.
    fn quoted(&mut self, close: u8, multiline: bool) {
        while self.at < self.src.len() {
            match self.src[self.at] {
                b'\\' => {
                    if self.peek(1) == b'\n' {
                        self.line += 1;
                    }
                    self.at += 2;
                }
                b'\n' => {
                    if !multiline {
                        return;
                    }
                    self.line += 1;
                    self.at += 1;
                }
                c if c == close => {
                    self.at += 1;
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    /// Template text after the backtick or a closing `}`: stops past the backtick, or past `${`
    /// with a new template level open.
    fn template(&mut self) {
        while self.at < self.src.len() {
            match self.src[self.at] {
                b'\\' => self.at += 2,
                b'\n' => {
                    self.line += 1;
                    self.at += 1;
                }
                b'`' => {
                    self.at += 1;
                    return;
                }
                b'$' if self.peek(1) == b'{' => {
                    self.at += 2;
                    self.templates.push(0);
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    /// A Rust raw string from its `r`: `r#*"..."#*`. False when it is not one.
    fn raw_string(&mut self) -> bool {
        let mut k = 1;
        while self.peek(k) == b'#' {
            k += 1;
        }
        if self.peek(k) != b'"' {
            return false;
        }
        let hashes = k - 1;
        self.at += k + 1;
        while self.at < self.src.len() {
            if self.src[self.at] == b'\n' {
                self.line += 1;
            } else if self.src[self.at] == b'"'
                && self.src[self.at + 1..].iter().take(hashes).filter(|b| **b == b'#').count()
                    == hashes
            {
                self.at += 1 + hashes;
                return true;
            }
            self.at += 1;
        }
        true
    }

    fn block_comment(&mut self) {
        let nests = self.dialect == Dialect::Rust;
        let mut depth = 0u32;
        while self.at < self.src.len() {
            match (self.src[self.at], self.peek(1)) {
                (b'/', b'*') => {
                    depth += 1;
                    self.at += 2;
                    if !nests && depth > 1 {
                        depth = 1;
                    }
                }
                (b'*', b'/') => {
                    self.at += 2;
                    depth -= 1;
                    if depth == 0 {
                        return;
                    }
                }
                (b'\n', _) => {
                    self.line += 1;
                    self.at += 1;
                }
                _ => self.at += 1,
            }
        }
    }

    /// Whether a `/` here starts a regex: by the token before it.
    fn regex_here(&self) -> bool {
        self.toks.last().is_none_or(|t| match t.kind {
            Kind::Lit => false,
            Kind::Punct(p) => !matches!(p, b')' | b']' | b'}'),
            Kind::Ident => {
                let word = &self.src[t.start as usize..t.end as usize];
                REGEX_AFTER.iter().any(|k| k.as_bytes() == word)
            }
        })
    }

    /// A regex literal from its `/`, to the closing `/` and flags. Gives up at a newline.
    const fn regex(&mut self) {
        self.at += 1;
        let mut class = false;
        while self.at < self.src.len() {
            match self.src[self.at] {
                b'\\' => self.at += 2,
                b'\n' => return,
                b'[' => {
                    class = true;
                    self.at += 1;
                }
                b']' => {
                    class = false;
                    self.at += 1;
                }
                b'/' if !class => {
                    self.at += 1;
                    while self.at < self.src.len() && is_ident(self.src[self.at]) {
                        self.at += 1;
                    }
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    /// Rust `'`: a char literal, or a lifetime or label (an identifier).
    fn rust_quote(&mut self, start: usize, line: u32) {
        let next = self.peek(1);
        if next == b'\\' {
            self.at += 1;
            self.quoted(b'\'', false);
            return self.push(Kind::Lit, start, line);
        }
        // One UTF-8 char, then a closing quote, makes a char literal.
        let width = match next {
            0..0x80 => 1,
            0xC0..0xE0 => 2,
            0xE0..0xF0 => 3,
            _ => 4,
        };
        if self.peek(1 + width) == b'\'' {
            self.at += 2 + width;
            return self.push(Kind::Lit, start, line);
        }
        self.at += 1;
        while self.at < self.src.len() && is_ident(self.src[self.at]) {
            self.at += 1;
        }
        self.push(Kind::Ident, start, line);
    }

    fn run(&mut self) {
        while self.at < self.src.len() {
            let c = self.src[self.at];
            let start = self.at;
            let line = self.line;
            match c {
                b'\n' => {
                    self.line += 1;
                    self.at += 1;
                }
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'/' if self.peek(1) == b'/' => {
                    while self.at < self.src.len() && self.src[self.at] != b'\n' {
                        self.at += 1;
                    }
                }
                b'/' if self.peek(1) == b'*' => self.block_comment(),
                b'/' if self.dialect == Dialect::Script && self.regex_here() => {
                    self.regex();
                    self.push(Kind::Lit, start, line);
                }
                b'"' => {
                    self.at += 1;
                    self.quoted(b'"', self.dialect == Dialect::Rust);
                    self.push(Kind::Lit, start, line);
                }
                b'\'' if self.dialect == Dialect::Rust => self.rust_quote(start, line),
                b'\'' => {
                    self.at += 1;
                    self.quoted(b'\'', false);
                    self.push(Kind::Lit, start, line);
                }
                b'`' if self.dialect == Dialect::Go => {
                    self.at += 1;
                    self.quoted_raw();
                    self.push(Kind::Lit, start, line);
                }
                b'`' if self.dialect == Dialect::Script => {
                    self.at += 1;
                    self.template();
                    self.push(Kind::Lit, start, line);
                }
                b'{' => {
                    if let Some(n) = self.templates.last_mut() {
                        *n += 1;
                    }
                    self.at += 1;
                    self.push(Kind::Punct(c), start, line);
                }
                b'}' if self.templates.last() == Some(&0) => {
                    self.templates.pop();
                    self.at += 1;
                    self.template();
                    self.push(Kind::Lit, start, line);
                }
                b'}' => {
                    if let Some(n) = self.templates.last_mut() {
                        *n -= 1;
                    }
                    self.at += 1;
                    self.push(Kind::Punct(c), start, line);
                }
                b'0'..=b'9' => {
                    while self.at < self.src.len()
                        && (is_ident(self.src[self.at])
                            || (self.src[self.at] == b'.' && self.peek(1).is_ascii_digit()))
                    {
                        self.at += 1;
                    }
                    self.push(Kind::Lit, start, line);
                }
                b'#' if self.dialect == Dialect::Script && is_ident_start(self.peek(1)) => {
                    self.at += 1;
                    self.ident(start, line);
                }
                c if is_ident_start(c) => {
                    if self.dialect == Dialect::Rust && self.rust_prefixed(start, line) {
                        continue;
                    }
                    self.ident(start, line);
                }
                _ => {
                    self.at += 1;
                    self.push(Kind::Punct(c), start, line);
                }
            }
        }
    }

    fn ident(&mut self, start: usize, line: u32) {
        while self.at < self.src.len() && is_ident(self.src[self.at]) {
            self.at += 1;
        }
        self.push(Kind::Ident, start, line);
    }

    /// Rust literals behind a prefix: `b"`, `b'`, `c"`, `r"`, `r#"`, `br#"`, and raw
    /// identifiers `r#name`. True when one was consumed.
    fn rust_prefixed(&mut self, start: usize, line: u32) -> bool {
        let (p0, p1) = (self.src[self.at], self.peek(1));
        let raw_at = match (p0, p1) {
            (b'r', b'#' | b'"') => Some(0),
            (b'b' | b'c', b'r') if matches!(self.peek(2), b'#' | b'"') => Some(1),
            _ => None,
        };
        if let Some(k) = raw_at {
            self.at += k;
            if self.raw_string() {
                self.push(Kind::Lit, start, line);
                return true;
            }
            self.at = start;
            if p1 == b'#' && is_ident_start(self.peek(2)) {
                self.at += 2;
                self.ident(start, line);
                return true;
            }
            return false;
        }
        match (p0, p1) {
            (b'b' | b'c', b'"') => {
                self.at += 2;
                self.quoted(b'"', true);
            }
            (b'b', b'\'') => {
                self.at += 2;
                self.quoted(b'\'', false);
            }
            _ => return false,
        }
        self.push(Kind::Lit, start, line);
        true
    }

    const fn quoted_raw(&mut self) {
        while self.at < self.src.len() {
            let c = self.src[self.at];
            self.at += 1;
            if c == b'\n' {
                self.line += 1;
            } else if c == b'`' {
                return;
            }
        }
    }
}

pub(crate) fn tokens(src: &str, dialect: Dialect) -> Vec<Tok> {
    let mut lx = Lexer {
        src: src.as_bytes(),
        at: 0,
        line: 1,
        dialect,
        toks: Vec::with_capacity(src.len() / 5),
        templates: Vec::new(),
    };
    lx.run();
    lx.toks
}
