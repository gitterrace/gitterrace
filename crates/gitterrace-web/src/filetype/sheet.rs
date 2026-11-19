//! xlsx and ods blobs: the first sheet as the CSV table (`table.rs`). Both formats are zip
//! files, so the declared inflated size of every member is summed from the central directory
//! first, and a workbook that would inflate past `INFLATED_CAP` is never opened.
//!
//! A small file can still declare a huge grid: ods repeats a row or a cell by count, and an
//! xlsx cell names any coordinate. The grid is bounded before anything is built at its size:
//! ods by a streaming count of content.xml before calamine reads it, xlsx by reading its cells
//! one at a time rather than through calamine's range, which allocates the bounding box.

use std::io::{BufReader, Cursor, Read};

use calamine::{Data, Ods, Reader as _, Xlsx, open_workbook_from_rs};
use quick_xml::events::{BytesStart, Event};

use crate::filetype::table::Table;

/// Bytes the members of a workbook may inflate to, in total: a zip bomb stops here. Not
/// configurable: a read bound on a file from the repository, which may be hostile.
const INFLATED_CAP: u64 = 64 << 20;

/// Cells a workbook may expand to, in any sheet ods holds or the xlsx sheet shown: about
/// 56 MB of calamine cells and formulas at the most. Not configurable, as `INFLATED_CAP`.
const CELL_CAP: u64 = 1 << 20;

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
/// when it does not read, would inflate past `cap` bytes, has more than `rows` rows or would
/// expand past `CELL_CAP` cells.
pub(crate) fn first_sheet(bytes: &[u8], cap: u64, rows: usize) -> Option<Table> {
    if inflated(bytes)? > cap {
        return None;
    }
    if let Ok(mut wb) = open_workbook_from_rs::<Xlsx<_>, _>(Cursor::new(bytes)) {
        return xlsx_sheet(&mut wb, rows);
    }
    let rows_cap = u64::try_from(rows).ok()?.saturating_add(1);
    if ods_grid(bytes)?.exceeds(rows_cap) {
        return None;
    }
    let mut wb = open_workbook_from_rs::<Ods<_>, _>(Cursor::new(bytes)).ok()?;
    let range = wb.worksheet_range_at(0)?.ok()?;
    if range.height() > rows.saturating_add(1) {
        return None;
    }
    let mut it = range
        .rows()
        .map(|r| r.iter().map(ToString::to_string).collect::<Vec<_>>());
    Some(split(&mut it))
}

/// `first_sheet` with the crate's caps.
pub(crate) fn table(bytes: &[u8]) -> Option<Table> {
    first_sheet(bytes, INFLATED_CAP, crate::filetype::table::ROW_CAP)
}

fn split(it: &mut impl Iterator<Item = Vec<String>>) -> Table {
    let head = it.next().unwrap_or_default();
    Table {
        head,
        rows: it.collect(),
    }
}

/// The xlsx's first sheet, its non-empty cells read one at a time and placed in their
/// bounding box only once that box is known to fit `rows` and `CELL_CAP`.
fn xlsx_sheet<RS: std::io::Read + std::io::Seek>(wb: &mut Xlsx<RS>, rows: usize) -> Option<Table> {
    let name = wb.sheet_names().into_iter().next()?;
    let mut r = wb.worksheet_cells_reader(&name).ok()?;
    let mut cells = Vec::new();
    while let Some(c) = r.next_cell().ok()? {
        if c.get_value() == &calamine::DataRef::Empty {
            continue;
        }
        if cells.len() as u64 >= CELL_CAP {
            return None;
        }
        cells.push((
            c.get_position(),
            Data::from(c.get_value().clone()).to_string(),
        ));
    }
    let Some(&((r0, c0), _)) = cells.first() else {
        return Some(split(&mut std::iter::empty()));
    };
    let (mut top, mut bottom, mut left, mut right) = (r0, r0, c0, c0);
    for &((row, col), _) in &cells {
        top = top.min(row);
        bottom = bottom.max(row);
        left = left.min(col);
        right = right.max(col);
    }
    let height = u64::from(bottom - top) + 1;
    let width = u64::from(right - left) + 1;
    let rows_cap = u64::try_from(rows).ok()?.saturating_add(1);
    if height > rows_cap || height.saturating_mul(width) > CELL_CAP {
        return None;
    }
    let (h, w) = (usize::try_from(height).ok()?, usize::try_from(width).ok()?);
    let mut grid = vec![vec![String::new(); w]; h];
    for ((row, col), v) in cells {
        grid[(row - top) as usize][(col - left) as usize] = v;
    }
    Some(split(&mut grid.into_iter()))
}

/// What calamine would build from an ods: the cells it pushes as it reads each row element
/// and the cells of its expanded ranges, summed over every sheet, since it reads them all.
/// Counted from above: a cell holding any value attribute counts as holding a value.
#[derive(Debug, Default, PartialEq, Eq)]
struct Grid {
    pushed: u64,
    expanded: u64,
    /// Rows of the tallest sheet, from its first row holding a value to its last.
    height: u64,
}

impl Grid {
    const fn exceeds(&self, rows: u64) -> bool {
        self.height > rows || self.pushed > CELL_CAP || self.expanded > CELL_CAP
    }
}

/// One sheet as it streams by: rows are counted, never stored.
#[derive(Default)]
struct Sheet {
    /// Rows read so far, and the row after the last holding a value, both with repeats.
    rows: u64,
    first: Option<u64>,
    end: u64,
    width: u64,
}

/// The `Grid` of the ods in `bytes`, streamed from content.xml. `None` when it does not read.
fn ods_grid(bytes: &[u8]) -> Option<Grid> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let f = z.by_name("content.xml").ok()?;
    count(BufReader::new(f.take(INFLATED_CAP)))
}

fn count(r: impl std::io::BufRead) -> Option<Grid> {
    let mut x = quick_xml::Reader::from_reader(r);
    x.config_mut().check_end_names = false;
    let (mut buf, mut g) = (Vec::new(), Grid::default());
    let mut sheet: Option<Sheet> = None;
    // The row element being read: its repeat count, cells so far, and its width up to and
    // including the last cell holding a value (calamine pushes no trailing empties).
    let (mut repeat, mut at, mut wide) = (0u64, 0u64, 0u64);
    loop {
        let ev = x.read_event_into(&mut buf).ok()?;
        let (start, e) = match &ev {
            Event::Start(e) => (true, e),
            Event::Empty(e) => (false, e),
            Event::End(e) => {
                match e.name().as_ref() {
                    "table:table-row" => end_row(sheet.as_mut()?, &mut g, repeat, wide),
                    "table:table" => end_sheet(&sheet.take()?, &mut g),
                    _ => {}
                }
                buf.clear();
                continue;
            }
            Event::Eof => return Some(g),
            _ => {
                buf.clear();
                continue;
            }
        };
        match e.name().as_ref() {
            "table:table" if start => sheet = Some(Sheet::default()),
            "table:table-row" => {
                (repeat, at, wide) = (repeats(e, "table:number-rows-repeated")?, 0, 0);
                if !start {
                    end_row(sheet.as_mut()?, &mut g, repeat, 0);
                }
            }
            "table:table-cell" | "table:covered-table-cell" => {
                at = at.saturating_add(repeats(e, "table:number-columns-repeated")?);
                if has_value(e) {
                    wide = at;
                }
            }
            _ => {}
        }
        buf.clear();
    }
}

fn end_row(s: &mut Sheet, g: &mut Grid, repeat: u64, wide: u64) {
    g.pushed = g.pushed.saturating_add(wide);
    if wide > 0 {
        s.first.get_or_insert(s.rows);
        s.end = s.rows.saturating_add(repeat);
        s.width = s.width.max(wide);
    }
    s.rows = s.rows.saturating_add(repeat);
}

fn end_sheet(s: &Sheet, g: &mut Grid) {
    let height = s.first.map_or(0, |f| s.end - f);
    g.expanded = g.expanded.saturating_add(height.saturating_mul(s.width));
    g.height = g.height.max(height);
}

/// The repeat count `attr` of `e`, 1 when absent, `None` when it is not a number.
fn repeats(e: &BytesStart<'_>, attr: &str) -> Option<u64> {
    match e.try_get_attribute(attr).ok()? {
        Some(a) => a.value.trim().parse().ok(),
        None => Some(1),
    }
}

/// Whether calamine could read a value or formula from the cell `e`.
fn has_value(e: &BytesStart<'_>) -> bool {
    e.attributes().with_checks(false).flatten().any(|a| {
        let k = a.key.as_ref();
        k == "table:formula" || k.starts_with("office:") || k == "calcext:value-type"
    })
}

#[cfg(test)]
#[path = "sheet_tests.rs"]
mod tests;
