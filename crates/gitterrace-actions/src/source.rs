//! The machine line every note and every Copy as markdown ends with.
//!
//! `source: <repo>@<sha>[:<path>[#L<a>-L<b>]]`. A tool that reads the note back finds the
//! exact place without parsing the prose. `static/send.js` builds the same line, and
//! `testdata/source-vectors.json` holds both to it.

use crate::payload::Lines;

/// The prefix that starts the line.
pub const SOURCE_PREFIX: &str = "source: ";

/// A parsed source line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub repo: String,
    pub sha: String,
    pub path: Option<String>,
    pub lines: Option<Lines>,
}

/// The line, no line break. One line selected is still `#La-La`, so the form never varies.
#[must_use]
pub fn source_line(repo: &str, sha: &str, path: Option<&str>, lines: Option<Lines>) -> String {
    // A line break in a name would end the line early, so control characters become spaces.
    let flat =
        |s: &str| -> String { s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect() };
    let mut out = format!("{SOURCE_PREFIX}{}@{sha}", flat(repo));
    if let Some(p) = path {
        out.push(':');
        // `%` and `#` encoded, so the only `#` on the line is the one before the range.
        out.push_str(&flat(p).replace('%', "%25").replace('#', "%23"));
        if let Some(l) = lines {
            out.push_str("#L");
            out.push_str(&l.start.to_string());
            out.push_str("-L");
            out.push_str(&l.end.to_string());
        }
    }
    out
}

/// Reads a line [`source_line`] wrote. The sha is the first `@` followed by hex up to a `:`
/// or the end, so a repo name may hold `@`; the path's `#` and `%` arrive as `%23` and `%25`.
#[must_use]
pub fn parse_source(line: &str) -> Option<Source> {
    let rest = line.strip_prefix(SOURCE_PREFIX)?;
    let (at, len) = rest.match_indices('@').find_map(|(i, _)| {
        let hex = rest[i + 1..].bytes().take_while(u8::is_ascii_hexdigit).count();
        let after = rest.as_bytes().get(i + 1 + hex);
        (hex >= 7 && matches!(after, None | Some(b':'))).then_some((i, hex))
    })?;
    let repo = &rest[..at];
    let sha = &rest[at + 1..at + 1 + len];
    let tail = &rest[at + 1 + len..];
    let (path, lines) = match tail.strip_prefix(':') {
        None => (None, None),
        Some(p) => match p.split_once('#') {
            None => (Some(decode(p)?), None),
            Some((p, f)) => (Some(decode(p)?), Some(fragment(f)?)),
        },
    };
    if repo.is_empty() || path.as_deref().is_some_and(str::is_empty) {
        return None;
    }
    Some(Source { repo: repo.to_owned(), sha: sha.to_owned(), path, lines })
}

/// The path as written: `%23` and `%25` back to `#` and `%`; any other escape is not ours.
fn decode(p: &str) -> Option<String> {
    let mut out = String::with_capacity(p.len());
    let mut rest = p;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let c = match rest.get(i + 1..i + 3)? {
            "23" => '#',
            "25" => '%',
            _ => return None,
        };
        out.push(c);
        rest = &rest[i + 3..];
    }
    out.push_str(rest);
    Some(out)
}

/// `La-Lb`, a <= b, both from 1.
fn fragment(f: &str) -> Option<Lines> {
    let (a, b) = f.strip_prefix('L')?.split_once("-L")?;
    let num = |s: &str| {
        (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
    };
    let (start, end) = (num(a)?, num(b)?);
    (start >= 1 && start <= end).then_some(Lines { start, end })
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{parse_source, source_line};
    use crate::payload::Lines;

    #[derive(Deserialize)]
    struct Vector {
        name: String,
        repo: String,
        sha: String,
        path: Option<String>,
        lines: Option<Lines>,
        expected: String,
    }

    /// The same file `make send-vectors` runs through send.js: both sides write each line
    /// byte for byte, and every line reads back to what wrote it.
    #[test]
    fn shared_vectors_hold_and_round_trip() {
        let text = include_str!("../testdata/source-vectors.json");
        let vectors: Vec<Vector> = serde_json::from_str(text).expect("vectors");
        assert!(vectors.len() >= 5);
        for v in &vectors {
            let line = source_line(&v.repo, &v.sha, v.path.as_deref(), v.lines);
            assert_eq!(line, v.expected, "{}", v.name);
            let back = parse_source(&line).expect(&v.name);
            assert_eq!(back.sha, v.sha, "{}", v.name);
            assert_eq!(back.path, v.path, "{}", v.name);
            assert_eq!(back.lines, v.lines, "{}", v.name);
        }
    }

    #[test]
    fn malformed_lines_do_not_parse() {
        for bad in
            ["source: r@xyz:p", "source: @abcdef12:p", "src: r@abcdef12", "source: r@abcdef12:"]
        {
            assert!(parse_source(bad).is_none(), "{bad}");
        }
        for bad in [
            "source: r@abcdef12:a#b#L1-L2",
            "source: r@abcdef12:a#L3-L2",
            "source: r@abcdef12:a%2F",
        ] {
            assert!(parse_source(bad).is_none(), "{bad}");
        }
        let one = Lines { start: 4, end: 4 };
        assert!(source_line("r", "abcdef12", Some("p"), Some(one)).ends_with("p#L4-L4"));
    }
}
