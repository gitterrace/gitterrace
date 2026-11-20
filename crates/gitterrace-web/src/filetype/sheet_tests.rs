use std::io::Write as _;

use proptest::prelude::*;

use super::*;

const OOXML: &str = "http://schemas.openxmlformats.org";
const CT: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml";

/// A one-sheet xlsx with `rows` rows of two cells, the first holding `<b>`.
fn xlsx(rows: usize) -> Vec<u8> {
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
    sheet_xlsx(&data)
}

/// A one-sheet xlsx whose `sheetData` holds `data`.
fn sheet_xlsx(data: &str) -> Vec<u8> {
    let rel = format!("{OOXML}/officeDocument/2006/relationships");
    let pkg = format!("{OOXML}/package/2006");
    let main = format!("{OOXML}/spreadsheetml/2006/main");
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
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
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

const OPENDOC: &str = "urn:oasis:names:tc:opendocument:xmlns";

/// An ods holding `sheet` (the inside of one `table:table`) as its only sheet.
fn ods(sheet: &str) -> Vec<u8> {
    let content = format!(
        "<office:document-content xmlns:office='{OPENDOC}:office:1.0' \
         xmlns:table='{OPENDOC}:table:1.0' xmlns:text='{OPENDOC}:text:1.0'><office:body>\
         <office:spreadsheet><table:table table:name='S'>{sheet}</table:table>\
         </office:spreadsheet></office:body></office:document-content>"
    );
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in [
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet".to_owned()),
        (
            "META-INF/manifest.xml",
            format!("<manifest:manifest xmlns:manifest='{OPENDOC}:manifest:1.0'/>"),
        ),
        ("content.xml", content),
    ] {
        w.start_file(name, o).expect("start");
        w.write_all(body.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

/// A row repeated `rows` times of `cols` cells, each a float when `full`, else empty.
fn row(rows: u64, cols: u64, full: bool) -> String {
    let v = if full { " office:value-type='float' office:value='7'" } else { "" };
    format!(
        "<table:table-row table:number-rows-repeated='{rows}'>\
         <table:table-cell table:number-columns-repeated='{cols}'{v}/></table:table-row>"
    )
}

#[test]
fn an_ods_sheet_reads_as_a_table() {
    let t = first_sheet(&ods(&row(3, 2, true)), 1 << 20, 10).expect("sheet");
    assert_eq!(t.head, ["7", "7"]);
    assert_eq!(t.rows.len(), 2);
}

#[test]
fn ods_repeated_rows_at_the_cap_read_and_one_more_is_refused() {
    assert_eq!(first_sheet(&ods(&row(4, 1, true)), 1 << 20, 3).map(|t| t.rows.len()), Some(3));
    assert!(first_sheet(&ods(&row(5, 1, true)), 1 << 20, 3).is_none());
}

/// Empty rows between two full ones expand; trailing empty rows and columns do not.
#[test]
fn ods_rows_count_from_the_first_value_to_the_last() {
    let g = |s: &str| ods_grid(&ods(s)).expect("grid");
    let gap = format!("{}{}{}", row(1, 2, true), row(1_000_000, 2, false), row(1, 2, true));
    assert_eq!(g(&gap).height, 1_000_002);
    assert!(first_sheet(&ods(&gap), 1 << 20, 10).is_none());
    let tail = format!("{}{}", row(2, 3, true), row(1_000_000, 16_384, false));
    assert_eq!(
        g(&tail),
        Grid {
            // calamine pushes a row element's cells once, whatever its repeat count.
            pushed: 3,
            expanded: 6,
            height: 2
        }
    );
    assert!(first_sheet(&ods(&tail), 1 << 20, 10).is_some());
}

#[test]
fn ods_repeated_columns_past_the_cell_cap_are_refused() {
    let wide = row(64, CELL_CAP / 64 + 1, true);
    assert!(ods_grid(&ods(&wide)).expect("grid").exceeds(10_000));
    assert!(first_sheet(&ods(&wide), 1 << 20, 10_000).is_none());
}

#[test]
fn a_bad_repeat_count_reads_nothing() {
    let bad = "<table:table-row table:number-rows-repeated='-1'><table:table-cell/>\
               </table:table-row>";
    assert!(first_sheet(&ods(bad), 1 << 20, 10).is_none());
}

#[test]
fn far_apart_xlsx_cells_are_refused_before_the_box_is_built() {
    let near = sheet_xlsx("<row r='1'><c r='A1'><v>1</v></c><c r='C1'><v>2</v></c></row>");
    assert_eq!(first_sheet(&near, 1 << 20, 10).expect("near").head, ["1", "", "2"]);
    let far = sheet_xlsx(
        "<row r='1'><c r='A1'><v>1</v></c></row>\
         <row r='1048576'><c r='XFD1048576'><v>2</v></c></row>",
    );
    assert!(first_sheet(&far, 1 << 20, 10).is_none());
    let wide = sheet_xlsx("<row r='1'><c r='A1'><v>1</v></c><c r='XFD1'><v>2</v></c></row>");
    assert_eq!(first_sheet(&wide, 1 << 20, 10).expect("wide").head.len(), 16_384);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any ods of repeated rows, full or empty, reads within the caps or not at all: never
    /// more rows than asked, never a panic, and refused exactly when its grid exceeds them.
    #[test]
    fn any_repeated_ods_stays_within_the_caps(
        rows in prop::collection::vec((0u64..2_000_000, 0u64..20_000, any::<bool>()), 1..6),
        cap in 1usize..50,
    ) {
        let sheet: String = rows.iter().map(|&(r, c, full)| row(r, c, full)).collect();
        let bytes = ods(&sheet);
        let grid = ods_grid(&bytes).expect("grid");
        match first_sheet(&bytes, 1 << 20, cap) {
            Some(t) => {
                prop_assert!(!grid.exceeds(cap as u64 + 1));
                prop_assert!(t.rows.len() <= cap);
            }
            None => prop_assert!(grid.exceeds(cap as u64 + 1) || grid.height == 0),
        }
    }

    /// Any two xlsx cells read as a box within the caps or not at all.
    #[test]
    fn any_two_xlsx_cells_stay_within_the_caps(
        a in (1u32..1_048_577, 0u32..16_384),
        b in (1u32..1_048_577, 0u32..16_384),
    ) {
        let cell = |(r, c): (u32, u32)| {
            let mut col = String::new();
            let mut n = c + 1;
            while n > 0 {
                col.insert(0, char::from(b'A' + ((n - 1) % 26) as u8));
                n = (n - 1) / 26;
            }
            format!("<row r='{r}'><c r='{col}{r}'><v>1</v></c></row>")
        };
        let (lo, hi) = if a.0 <= b.0 { (a, b) } else { (b, a) };
        let t = first_sheet(&sheet_xlsx(&(cell(lo) + &cell(hi))), 1 << 20, 100);
        let h = u64::from(hi.0 - lo.0) + 1;
        let w = u64::from(a.1.abs_diff(b.1)) + 1;
        prop_assert_eq!(t.is_some(), h <= 101 && h * w <= CELL_CAP);
    }

    /// Bytes mutated from a valid workbook never panic, and what reads respects the caps.
    #[test]
    fn mutated_workbooks_never_panic(
        flips in prop::collection::vec((any::<usize>(), any::<u8>()), 1..8),
        which in any::<bool>(),
    ) {
        let mut b = if which { xlsx(4) } else { ods(&row(4, 3, true)) };
        for (at, v) in flips {
            let n = b.len();
            b[at % n] = v;
        }
        if let Some(t) = first_sheet(&b, 1 << 20, 5) {
            prop_assert!(t.rows.len() <= 5);
        }
    }
}
