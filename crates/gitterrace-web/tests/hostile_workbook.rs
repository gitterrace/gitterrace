//! A tiny workbook that declares a huge grid renders its blob page without building that
//! grid: an ods row repeated a million times, and an xlsx naming two far-apart cells. Peak
//! heap is read from a counting global allocator, so this file holds this one test only.
#![expect(clippy::expect_used, reason = "setup failures should panic")]

use std::alloc::System;
use std::io::{Cursor, Write as _};

use cap::Cap;

use gitterrace_testkit::{Fixture, page};

#[global_allocator]
static ALLOC: Cap<System> = Cap::new(System, usize::MAX);

/// Peak heap the page may add: far below the grid either workbook declares.
const PEAK_MB: usize = 48;

fn zip_of(parts: &[(&str, String)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, body) in parts {
        w.start_file(*name, o).expect("start");
        w.write_all(body.as_bytes()).expect("write");
    }
    w.finish().expect("finish").into_inner()
}

/// An ods whose one sheet repeats a row of 16 cells, each holding a value, a million times.
fn repeated_ods() -> Vec<u8> {
    let office = "urn:oasis:names:tc:opendocument:xmlns";
    zip_of(&[
        (
            "mimetype",
            "application/vnd.oasis.opendocument.spreadsheet".into(),
        ),
        (
            "META-INF/manifest.xml",
            format!("<manifest:manifest xmlns:manifest='{office}:manifest:1.0'/>"),
        ),
        (
            "content.xml",
            format!(
                "<office:document-content xmlns:office='{office}:office:1.0' \
                 xmlns:table='{office}:table:1.0'><office:body><office:spreadsheet>\
                 <table:table table:name='S'>\
                 <table:table-row table:number-rows-repeated='1000000'>\
                 <table:table-cell table:number-columns-repeated='16' \
                 office:value-type='float' office:value='1'/></table:table-row>\
                 </table:table></office:spreadsheet></office:body></office:document-content>"
            ),
        ),
    ])
}

/// An xlsx with two cells at opposite corners of a 1048576 by 16384 sheet.
fn sparse_xlsx() -> Vec<u8> {
    let ooxml = "http://schemas.openxmlformats.org";
    let ct = "application/vnd.openxmlformats-officedocument.spreadsheetml";
    let rel = format!("{ooxml}/officeDocument/2006/relationships");
    let pkg = format!("{ooxml}/package/2006");
    let main = format!("{ooxml}/spreadsheetml/2006/main");
    zip_of(&[
        (
            "[Content_Types].xml",
            format!(
                "<Types xmlns='{pkg}/content-types'>\
                 <Default Extension='xml' ContentType='application/xml'/>\
                 <Override PartName='/xl/workbook.xml' ContentType='{ct}.sheet.main+xml'/>\
                 <Override PartName='/xl/worksheets/sheet1.xml' \
                 ContentType='{ct}.worksheet+xml'/></Types>"
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
            format!(
                "<worksheet xmlns='{main}'><sheetData>\
                 <row r='1'><c r='A1'><v>1</v></c></row>\
                 <row r='1048576'><c r='XFD1048576'><v>2</v></c></row>\
                 </sheetData></worksheet>"
            ),
        ),
    ])
}

#[tokio::test]
async fn a_workbook_declaring_a_huge_grid_renders_without_building_it() {
    let dir = tempfile::tempdir().expect("tmp");
    let f = Fixture::init(dir.path());
    std::fs::write(dir.path().join("rows.ods"), repeated_ods()).expect("write");
    std::fs::write(dir.path().join("far.xlsx"), sparse_xlsx()).expect("write");
    let f = f.commit("feat: workbooks");
    let router = gitterrace_web::app_router(f.path()).expect("router");
    for name in ["rows.ods", "far.xlsx"] {
        let before = ALLOC.allocated();
        let (status, body) = page(&router, &format!("/blob/HEAD/{name}")).await;
        let peak = ALLOC.max_allocated().saturating_sub(before) >> 20;
        assert_eq!(status, 200, "{name}");
        assert!(
            !body.contains("class=\"data-table\""),
            "{name}: rendered a table"
        );
        assert!(peak < PEAK_MB, "{name}: peak heap grew {peak} MB");
    }
}
