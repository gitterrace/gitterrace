//! Text to terms, and the quoting the serializer writes back.

use gitterrace_types::{Period, StringEnum};

use crate::query::spec::{KEYWORDS, UNSUPPORTED};
use crate::query::{Leaf, QueryError, Term, When};

/// What the field supports, for the errors that refuse anything else.
const SUPPORTED: &str = "terms side by side (all must match), `!` before a term, and \
                         commas inside one value (any of them)";

/// Longest filter text read: a URL parameter, bounded so a parse stays cheap. Not configurable:
/// no real filter comes near it, and the API and pages must refuse the same text.
const MAX_LEN: usize = 2_048;

pub(super) fn terms(text: &str) -> Result<Vec<Term>, QueryError> {
    if text.len() > MAX_LEN {
        return Err(QueryError {
            start: MAX_LEN,
            end: text.len(),
            message: format!("the filter is longer than {MAX_LEN} bytes"),
        });
    }
    let mut s = Scan { text, at: 0 };
    let mut out = Vec::new();
    loop {
        s.skip_space();
        if s.done() {
            return Ok(out);
        }
        out.push(s.term()?);
    }
}

/// `v` as one value item: bare when it reads back as itself, else quoted.
pub(super) fn quote(v: &str) -> String {
    let bare = !v.is_empty()
        && !v.starts_with('!')
        && !v.chars().any(|c| c.is_whitespace() || "\",|()\\".contains(c));
    if bare { v.to_owned() } else { quoted(v) }
}

/// `v` as message text: bare only for a plain word that is no keyword.
pub(super) fn quote_text(v: &str) -> String {
    let bare = !v.is_empty()
        && v != "merge"
        && v.chars().all(|c| c.is_alphanumeric() || "-_./#@'".contains(c))
        && !v.starts_with(['-', '!']);
    if bare { v.to_owned() } else { quoted(v) }
}

fn quoted(v: &str) -> String {
    let mut q = String::with_capacity(v.len() + 2);
    q.push('"');
    for c in v.chars() {
        if c == '"' || c == '\\' {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    q
}

struct Scan<'a> {
    text: &'a str,
    at: usize,
}

impl Scan<'_> {
    const fn done(&self) -> bool {
        self.at >= self.text.len()
    }

    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn err(&self, start: usize, message: String) -> QueryError {
        QueryError {
            start,
            end: self.at.max(start + 1).min(self.text.len().max(start + 1)),
            message,
        }
    }

    fn term(&mut self) -> Result<Term, QueryError> {
        let start = self.at;
        let neg = self.peek() == Some('!');
        if neg {
            self.bump();
        }
        match self.peek() {
            Some(c @ ('|' | '(' | ')')) => {
                self.bump();
                return Err(
                    self.err(start, format!("`{c}` is not supported yet. Supported: {SUPPORTED}."))
                );
            }
            Some('"') => {
                let text = self.quoted()?;
                return Ok(Term { neg, leaf: Leaf::Text(text) });
            }
            None => return Err(self.err(start, "`!` needs a term after it".to_owned())),
            Some(c) if c.is_whitespace() => {
                return Err(self.err(start, "`!` needs a term after it".to_owned()));
            }
            Some(_) => {}
        }
        let word_at = self.at;
        while self.peek().is_some_and(|c| !c.is_whitespace() && !"\":|()".contains(c)) {
            self.bump();
        }
        let word = &self.text[word_at..self.at];
        if self.peek() != Some(':') {
            if matches!(self.peek(), Some('|' | '(' | ')' | '"')) {
                return Err(self.err(
                    self.at,
                    format!("unexpected `{}`. Supported: {SUPPORTED}.", self.peek().unwrap_or(' ')),
                ));
            }
            let leaf = if word == "merge" { Leaf::Merge } else { Leaf::Text(word.to_owned()) };
            return Ok(Term { neg, leaf });
        }
        self.bump();
        self.keyed(start, neg, word)
    }

    /// The value of `word:`, the colon just read.
    fn keyed(&mut self, start: usize, neg: bool, word: &str) -> Result<Term, QueryError> {
        if let Some((_, why)) = UNSUPPORTED.iter().find(|(k, _)| *k == word) {
            return Err(self.err(start, format!("{why}.")));
        }
        let leaf = match word {
            "type" => Leaf::Kind(self.list(start)?),
            "scope" => Leaf::Scope(self.list(start)?),
            "author" => Leaf::Author(self.list(start)?),
            "path" => {
                let v: Vec<String> =
                    self.list(start)?.into_iter().map(|p| p.trim_matches('/').to_owned()).collect();
                if v.iter().any(String::is_empty) {
                    return Err(self.err(start, "`path:` needs a path".to_owned()));
                }
                Leaf::Path(v)
            }
            "after" | "before" => {
                let v = self.value(start)?;
                let Some(w) = bound(&v) else {
                    return Err(self.err(
                        start,
                        format!(
                            "`{word}:` takes {} or a date YYYY-MM-DD, got {v:?}",
                            Period::expected()
                        ),
                    ));
                };
                if word == "before" { Leaf::Before(w) } else { Leaf::After(w) }
            }
            _ => {
                let known: Vec<&str> =
                    KEYWORDS.iter().filter(|k| k.syntax.contains(':')).map(|k| k.keyword).collect();
                return Err(self.err(
                    start,
                    format!(
                        "unknown keyword `{word}:`. Known: {}. Quote text with a colon.",
                        known.join(", ")
                    ),
                ));
            }
        };
        Ok(Term { neg, leaf })
    }

    /// A comma list of values, at least one, none empty.
    fn list(&mut self, start: usize) -> Result<Vec<String>, QueryError> {
        let mut out = vec![self.value(start)?];
        while self.peek() == Some(',') {
            self.bump();
            out.push(self.value(start)?);
        }
        Ok(out)
    }

    fn value(&mut self, start: usize) -> Result<String, QueryError> {
        if self.peek() == Some('"') {
            let v = self.quoted()?;
            if v.is_empty() {
                return Err(self.err(start, "empty value".to_owned()));
            }
            return Ok(v);
        }
        let at = self.at;
        while self.peek().is_some_and(|c| !c.is_whitespace() && !",\"|()".contains(c)) {
            self.bump();
        }
        if at == self.at {
            return Err(self.err(start, "the keyword needs a value after `:`".to_owned()));
        }
        Ok(self.text[at..self.at].to_owned())
    }

    /// A `"..."` string with `\"` and `\\` escapes.
    fn quoted(&mut self) -> Result<String, QueryError> {
        let start = self.at;
        self.bump();
        let mut v = String::new();
        loop {
            match self.bump() {
                None => return Err(self.err(start, "unclosed quote".to_owned())),
                Some('"') => return Ok(v),
                Some('\\') => match self.bump() {
                    Some(c) => v.push(c),
                    None => return Err(self.err(start, "unclosed quote".to_owned())),
                },
                Some(c) => v.push(c),
            }
        }
    }
}

/// The bound `v` names: a period other than `all`, or a day.
fn bound(v: &str) -> Option<When> {
    match Period::parse(v) {
        Some(Period::All) => None,
        Some(p) => Some(When::Period(p)),
        None => crate::parse_day(v).map(When::Day),
    }
}
