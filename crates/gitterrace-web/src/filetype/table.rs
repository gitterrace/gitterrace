//! CSV and TSV blobs as a table: parsed per RFC 4180 (quoted fields, embedded commas
//! and newlines) by the `csv` crate, capped: a longer file shows as source.

use askama::Template;

/// Rows past this many render as source instead: a table that long helps no one and costs
/// the page its paint. Not configurable: past it the source view is the better page anyway.
pub(crate) const ROW_CAP: usize = 5000;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Table {
    pub(crate) head: Vec<String>,
    pub(crate) rows: Vec<Vec<String>>,
}

#[derive(Template)]
#[template(path = "data_table.html")]
struct TableMarkup<'a> {
    name: &'a str,
    t: &'a Table,
}

/// The delimiter `path` is split on, or `None` for a file that is not tabular.
pub(crate) fn delimiter(path: &str) -> Option<u8> {
    let ext = crate::fmt::ext(path)?;
    match ext.as_str() {
        "csv" => Some(b','),
        "tsv" | "tab" => Some(b'\t'),
        _ => None,
    }
}

/// `text` as a table whose first record is the header, or `None` when it does not parse or
/// has more than `cap` rows.
pub(crate) fn parse(text: &str, delim: u8, cap: usize) -> Option<Table> {
    let mut r = csv::ReaderBuilder::new()
        .delimiter(delim)
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut records = r.records();
    let head = records.next()?.ok()?.iter().map(str::to_owned).collect();
    let mut rows = Vec::new();
    for rec in records {
        if rows.len() == cap {
            return None;
        }
        rows.push(rec.ok()?.iter().map(str::to_owned).collect());
    }
    Some(Table { head, rows })
}

/// The table's markup, every cell escaped by the template.
pub(crate) fn html(name: &str, t: &Table) -> String {
    TableMarkup { name, t }.render().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quoted_comma_and_newline_stay_in_one_field() {
        let t = parse("a,b\n\"x, y\",\"line1\nline2\"\n", b',', 10).expect("table");
        assert_eq!(t.head, ["a", "b"]);
        assert_eq!(t.rows, [vec!["x, y".to_owned(), "line1\nline2".to_owned()]]);
    }

    #[test]
    fn tabs_split_a_tsv() {
        let t = parse("a\tb\n1\t2\n", b'\t', 10).expect("table");
        assert_eq!(t.rows, [vec!["1".to_owned(), "2".to_owned()]]);
    }

    #[test]
    fn more_rows_than_the_cap_show_as_source() {
        assert!(parse("h\n1\n2\n", b',', 2).is_some());
        assert!(parse("h\n1\n2\n3\n", b',', 2).is_none());
    }

    #[test]
    fn cells_are_escaped() {
        let t = parse("<b>\n<script>x</script>\n", b',', 10).expect("table");
        let h = html("t.csv", &t);
        assert!(!h.contains("<script>") && h.contains("&#60;script&#62;"), "{h}");
    }

    #[test]
    fn only_csv_and_tsv_are_tabular() {
        assert_eq!(delimiter("a/B.CSV"), Some(b','));
        assert_eq!(delimiter("x.tsv"), Some(b'\t'));
        assert_eq!(delimiter("x.txt"), None);
    }
}
