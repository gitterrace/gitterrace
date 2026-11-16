//! xlsx and ods blobs: the first sheet as the CSV table (`table.rs`). Both formats are zip
//! files, so the declared inflated size of every member is summed from the central directory
//! first, and a workbook that would inflate past `INFLATED_CAP` is never opened.

use std::io::Cursor;

use calamine::{Reader as _, open_workbook_auto_from_rs};

use crate::filetype::table::Table;

/// Bytes the members of a workbook may inflate to, in total: a zip bomb stops here. Not
/// configurable: a read bound on a file from the repository, which may be hostile.
const INFLATED_CAP: u64 = 64 << 20;

/// Whether `path` is a workbook this module reads.
pub(crate) fn is_sheet(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case("xlsx") || e.eq_ignore_ascii_case("ods"))
}

/// The declared inflated size of every member, or `None` when the zip does not read.
fn inflated(bytes: &[u8]) -> Option<u64> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut sum = 0u64;
    for i in 0..z.len() {
        sum = sum.saturating_add(z.by_index_raw(i).ok()?.size());
    }
    Some(sum)
}

/// The first sheet of the workbook in `bytes` as a table, its first row the header. `None`
/// when it does not read, would inflate past `cap` bytes or has more than `rows` rows.
pub(crate) fn first_sheet(bytes: &[u8], cap: u64, rows: usize) -> Option<Table> {
    if inflated(bytes)? > cap {
        return None;
    }
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes)).ok()?;
    let range = wb.worksheet_range_at(0)?.ok()?;
    if range.height() > rows.saturating_add(1) {
        return None;
    }
    let mut it = range
        .rows()
        .map(|r| r.iter().map(ToString::to_string).collect::<Vec<_>>());
    let head = it.next().unwrap_or_default();
    Some(Table {
        head,
        rows: it.collect(),
    })
}

/// `first_sheet` with the crate's caps.
pub(crate) fn table(bytes: &[u8]) -> Option<Table> {
    first_sheet(bytes, INFLATED_CAP, crate::filetype::table::ROW_CAP)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use super::*;

    const OOXML: &str = "http://schemas.openxmlformats.org";
    const CT: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml";

    /// A one-sheet xlsx with `rows` rows of two cells, the first holding `<b>`.
    fn xlsx(rows: usize) -> Vec<u8> {
        let rel = format!("{OOXML}/officeDocument/2006/relationships");
        let pkg = format!("{OOXML}/package/2006");
        let main = format!("{OOXML}/spreadsheetml/2006/main");
        let mut data = String::new();
        for r in 1..=rows {
            std::fmt::Write::write_fmt(
                &mut data,
                format_args!(
                    "<row r='{r}'><c r='A{r}' t='inlineStr'><is><t>&lt;b&gt;{r}</t></is></c>\
                     <c r='B{r}'><v>{r}</v></c></row>"
                ),
            )
            .expect("fmt");
        }
        let parts = [
            (
                "[Content_Types].xml",
                format!(
                    "<Types xmlns='{pkg}/content-types'>\
                     <Default Extension='xml' ContentType='application/xml'/>\
                     <Override PartName='/xl/workbook.xml' ContentType='{CT}.sheet.main+xml'/>\
                     <Override PartName='/xl/worksheets/sheet1.xml' \
                     ContentType='{CT}.worksheet+xml'/></Types>"
                ),
            ),
            (
                "_rels/.rels",
                format!(
                    "<Relationships xmlns='{pkg}/relationships'><Relationship Id='r1' \
                     Type='{rel}/officeDocument' Target='xl/workbook.xml'/></Relationships>"
                ),
            ),
            (
                "xl/workbook.xml",
                format!(
                    "<workbook xmlns='{main}' xmlns:r='{rel}'><sheets>\
                     <sheet name='S' sheetId='1' r:id='r1'/></sheets></workbook>"
                ),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                format!(
                    "<Relationships xmlns='{pkg}/relationships'><Relationship Id='r1' \
                     Type='{rel}/worksheet' Target='worksheets/sheet1.xml'/></Relationships>"
                ),
            ),
            (
                "xl/worksheets/sheet1.xml",
                format!("<worksheet xmlns='{main}'><sheetData>{data}</sheetData></worksheet>"),
            ),
        ];
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in parts {
            w.start_file(name, o).expect("start");
            w.write_all(body.as_bytes()).expect("write");
        }
        w.finish().expect("finish").into_inner()
    }

    #[test]
    fn the_first_sheet_reads_as_a_table() {
        let t = first_sheet(&xlsx(3), 1 << 20, 10).expect("sheet");
        assert_eq!(t.head, ["<b>1", "1"]);
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.rows[1], ["<b>3", "3"]);
    }

    #[test]
    fn a_workbook_inflating_past_the_cap_is_never_opened() {
        let x = xlsx(3);
        assert!(first_sheet(&x, 1 << 20, 10).is_some());
        assert!(first_sheet(&x, 100, 10).is_none());
    }

    #[test]
    fn more_rows_than_the_cap_is_refused() {
        assert!(first_sheet(&xlsx(4), 1 << 20, 3).is_some());
        assert!(first_sheet(&xlsx(5), 1 << 20, 3).is_none());
    }

    #[test]
    fn garbage_reads_nothing_and_never_panics() {
        assert!(table(b"PK\x03\x04junk").is_none());
        assert!(table(b"").is_none());
    }

    /// The blob page shows a committed xlsx as a table, every cell escaped.
    #[tokio::test]
    async fn an_xlsx_blob_renders_its_first_sheet() {
        let dir = tempfile::tempdir().expect("tmp");
        let f = gitterrace_testkit::Fixture::init(dir.path());
        std::fs::write(dir.path().join("book.xlsx"), xlsx(2)).expect("write");
        let f = f.commit("feat: book");
        let router = crate::app_router(f.path()).expect("router");
        let (status, body) = gitterrace_testkit::page(&router, "/blob/HEAD/book.xlsx").await;
        assert_eq!(status, 200);
        assert!(body.contains("class=\"data-table\""), "no table");
        assert!(body.contains("<td>&#60;b&#62;2</td>"), "no escaped cell");
    }

    #[test]
    fn only_xlsx_and_ods_are_sheets() {
        assert!(is_sheet("a/B.XLSX") && is_sheet("x.ods"));
        assert!(!is_sheet("x.xls") && !is_sheet("x.csv"));
    }
}
