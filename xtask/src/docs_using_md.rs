//! Generated blocks of `docs/using.md`.

use std::fmt::Write as _;

/// The commit filter's keywords, from the table the parser and the field's completions read.
pub(crate) fn log_filter() -> String {
    let mut s = String::from("| Syntax | Matches |\n| --- | --- |\n");
    for k in gitterrace_index::query::KEYWORDS {
        let _ = writeln!(s, "| `{}` | {} |", k.syntax.replace('|', "\\|"), k.description);
    }
    s.push_str("\nA period is `1w`, `1m`, `3m` or `1y`, counted back from the newest commit.\n");
    s
}
