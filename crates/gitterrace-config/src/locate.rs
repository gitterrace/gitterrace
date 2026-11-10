//! Where in the file a config error is: the line and column the settings page marks. A TOML
//! syntax error carries its byte offset; a bad value names its key, found again in the text.

use toml_edit::{Document, Item};

use crate::ConfigError;

/// One error at its place in the text. Line and column count from 1; 0 when the error has no
/// place (a value the file never wrote, such as a default made invalid by another key).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("line {line}: {msg}")]
pub struct Located {
    pub line: usize,
    pub col: usize,
    /// The key the error names, `section.name`, when it names one.
    pub key: Option<String>,
    pub msg: String,
}

/// Places `e`, an error from loading `text`.
pub fn located(text: &str, e: &ConfigError) -> Located {
    let (at, key) = match e {
        ConfigError::Parse { at, .. } => (*at, None),
        ConfigError::Expected { key, .. } | ConfigError::Invalid { key, .. } => {
            (key_offset(text, key), Some(key.clone()))
        }
        ConfigError::Read { .. } | ConfigError::File { .. } => (None, None),
    };
    let (line, col) = at.map_or((0, 0), |o| line_col(text, o));
    Located {
        line,
        col,
        key,
        msg: e.to_string(),
    }
}

/// Whether `text` is TOML at all, with the place of the first syntax error. For a file whose own
/// loader reports no place (`actions.toml`), so its syntax errors still name a line.
pub fn syntax(text: &str) -> Result<(), Located> {
    Document::parse(text).map(drop).map_err(|e| {
        let (line, col) = e.span().map_or((0, 0), |s| line_col(text, s.start));
        Located {
            line,
            col,
            key: None,
            msg: e.message().to_owned(),
        }
    })
}

/// 1-based line and column of byte `offset`.
pub(crate) fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = text.get(..offset.min(text.len())).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

/// Byte offset of the deepest part of the dotted `key` the text writes: the value of
/// `server.addr`, else the `[server]` header. `None` when not even the section is there.
fn key_offset(text: &str, key: &str) -> Option<usize> {
    let doc = Document::parse(text).ok()?;
    let mut item = doc.as_item();
    let mut best = None;
    for part in key.split('.') {
        let Some(next) = item.get(part) else { break };
        let span = match next {
            Item::Table(t) => t.span(),
            _ => item
                .as_table_like()
                .and_then(|t| t.key(part))
                .and_then(toml_edit::Key::span)
                .or_else(|| next.span()),
        };
        best = span.map(|s| s.start).or(best);
        item = next;
    }
    best
}

#[cfg(test)]
mod tests {
    use crate::locate::{line_col, located, syntax};
    use crate::parse;

    fn place(text: &str) -> (usize, usize, Option<String>) {
        let e = parse(text).expect_err("bad");
        let l = located(text, &e);
        (l.line, l.col, l.key)
    }

    /// The parse and the plain syntax check both name the broken line.
    #[test]
    fn a_syntax_error_names_its_line() {
        let (line, _, key) = place("[watch]\nenabled = true\nrescan_secs = = 5\n");
        assert_eq!((line, key), (3, None));
        assert_eq!(syntax("a = 1\n\nb = \n").expect_err("bad").line, 3);
        syntax("version = 1\n[[action]]\nid = \"x\"\n").expect("toml");
    }

    #[test]
    fn a_bad_value_names_the_line_of_its_key() {
        let text = "# top\n[server]\nlive_streams = 4\n\naddr = \"nope\"\n";
        assert_eq!(place(text), (5, 1, Some("server.addr".into())));
        let text = "[watch]\n\n  rescan_secs = 1\n";
        assert_eq!(place(text), (3, 3, Some("watch.rescan_secs".into())));
        let text = "[repo.x.features]\nlive = true\n";
        assert_eq!(place(text).0, 2);
    }

    #[test]
    fn columns_count_characters() {
        assert_eq!(line_col("ab\ncäd", 6), (2, 3));
        assert_eq!(line_col("", 0), (1, 1));
    }
}
