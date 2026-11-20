//! The Graph view of the log: one small SVG beside each commit row, drawn on the server from
//! the columns `gitterrace_index::graph` lays out. A page replays the rows above it, so the
//! lines leaving one page are the lines entering the next. The drawing is decorative
//! (`aria-hidden`): the row's text carries the meaning.
//!
//! A cell is two SVGs. The lines stretch to the row's height (`preserveAspectRatio="none"`,
//! strokes kept even by `vector-effect`), the dot keeps its shape in a strip centred on the
//! row, so a row that wraps to two lines on a phone still meets its neighbours.

use std::collections::HashMap;

use askama::Template;
use askama::filters::HtmlSafe;
use gitterrace_index::graph::{FAR_ROWS, Open, Parent, Placed, Row, Seg};

/// Width of one column in CSS pixels.
const COL: u32 = 12;
/// Columns shown before the rest fold into a "+N" marker: a narrow screen, then a wide one.
/// app.css shows the marker that matches the screen, and hides the pieces of line and the dots
/// classed `xn` (past the narrow cap) or `xw` (past the wide one), so a line past the cap is
/// never drawn cut off at the cell's edge.
const FOLDS: [(u16, &str); 2] = [(3, "n"), (8, "w")];
/// Height of the line SVG's coordinate space: a row's top edge is 0, its middle 50.
const H: u32 = 100;

/// The rows `from..to` of `history`, laid out with the lines of the rows above carried in.
/// `history` is the log's order as `(id, parents)` and should reach [`FAR_ROWS`] past `to`, so
/// a parent missing from it is far for certain. A parent listed above its child (clock skew
/// in the time order) is drawn as a stub too: a line never runs upwards.
pub(crate) fn place(history: &[(String, Vec<String>)], from: usize, to: usize) -> Vec<Placed> {
    let mut pos: HashMap<&str, u32> = HashMap::with_capacity(history.len());
    for (i, (id, _)) in history.iter().enumerate() {
        let i = u32::try_from(i).unwrap_or(u32::MAX);
        pos.entry(id.as_str()).or_insert(i);
    }
    let mut open = Open::default();
    let mut out = Vec::with_capacity(to.saturating_sub(from));
    for (i, (_, parents)) in history.iter().enumerate().take(to) {
        let here = u32::try_from(i).unwrap_or(u32::MAX);
        let row = Row {
            id: here,
            parents: parents
                .iter()
                .map(|p| match pos.get(p.as_str()) {
                    Some(&at) if at > here => Parent { id: at, far: at - here > FAR_ROWS },
                    _ => Parent { id: u32::MAX, far: true },
                })
                .collect(),
        };
        let placed = open.place(&row);
        if i >= from {
            out.push(placed);
        }
    }
    out
}

/// Columns the widest row of `rows` uses.
pub(crate) fn columns(rows: &[Placed]) -> u16 {
    rows.iter().map(|p| reach(p) + 1).max().unwrap_or(1)
}

/// The rightmost column a row draws in.
fn reach(p: &Placed) -> u16 {
    p.top.iter().chain(&p.bottom).flat_map(|s| [s.from, s.to]).chain([p.col]).max().unwrap_or(0)
}

/// One piece of line: its SVG path data and classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Line {
    pub(crate) d: String,
    /// `main` on the first-parent chain, plus the fold that hides it (see [`hide`]).
    pub(crate) cls: &'static str,
    /// The column's color class, from the cycle of theme colors.
    pub(crate) k: &'static str,
}

/// Column colors, cycling: column 0's line is `k0` (the accent), every other line takes the
/// next class in turn. graph.css maps each to an existing theme token.
const LANES: [&str; 6] = ["k0", "k1", "k2", "k3", "k4", "k5"];

/// The color class of column `color`.
pub(crate) fn lane(color: u32) -> &'static str {
    let i = match color {
        0 => 0,
        c => 1 + (c - 1) % 5,
    };
    LANES.get(usize::try_from(i).unwrap_or(0)).copied().unwrap_or("k0")
}

/// One row's cell, as `graph_cell.html` draws it.
#[derive(Debug, Clone, PartialEq, Eq, Template)]
#[template(path = "graph_cell.html")]
pub(crate) struct Cell {
    /// Width in CSS pixels.
    pub(crate) w: u32,
    pub(crate) paths: Vec<Line>,
    /// x of a stub below the dot, for edges to far parents.
    pub(crate) stub: Option<u32>,
    /// The dot's classes: `m` for a merge (hollow), `main` on the first-parent chain.
    pub(crate) dot: &'static str,
    /// The fold that hides the dot and its stub, `""` when every screen shows them.
    pub(crate) off: &'static str,
    pub(crate) cx: u32,
    /// The dot's column color class, also worn by the tip's labels.
    pub(crate) lane: &'static str,
    /// A branch tip with labels: a dashed line runs from the cell's left edge to the dot.
    pub(crate) tip: bool,
    /// `(x, classes)` ends of the tip's dashed line: the dot, hidden where a fold hides it,
    /// and for each such fold a line to that fold's edge shown only on its screen.
    pub(crate) reach: Vec<(u32, &'static str)>,
    /// Screens (`n`, `w`) whose fold hides the dot: there a muted mark at the fold edge
    /// anchors the row instead.
    pub(crate) edge: Vec<&'static str>,
    /// `(screen class, hidden columns)` for each fold whose count differs from the row above.
    pub(crate) folds: Vec<(&'static str, u16)>,
}

impl HtmlSafe for Cell {}

/// The cell of one row, `width` columns wide. `merge` draws the dot hollow. A "+N" count
/// shows only where it differs from `above`, the row drawn just over this one, so a run of
/// rows with the same hidden columns reads as one count.
pub(crate) fn cell(p: &Placed, above: Option<&Placed>, width: u16, merge: bool) -> Cell {
    let was = above.map(hidden);
    let halves = p.top.iter().map(|s| (*s, 0)).chain(p.bottom.iter().map(|s| (*s, H / 2)));
    Cell {
        w: u32::from(width) * COL,
        paths: halves.map(|(seg, y0)| line(seg, y0)).collect(),
        stub: (p.stubs > 0).then(|| x(p.col)),
        dot: match (merge, p.color == 0) {
            (true, true) => "m main",
            (true, false) => "m",
            (false, true) => "main",
            (false, false) => "",
        },
        off: hide(p.col),
        edge: FOLDS.iter().filter(|&&(keep, _)| p.col >= keep).map(|&(_, cls)| cls).collect(),
        cx: x(p.col),
        lane: lane(p.color),
        tip: false,
        reach: tip_ends(p.col),
        folds: hidden(p)
            .into_iter()
            .zip(FOLDS)
            .enumerate()
            .filter(|&(i, (n, _))| n > 0 && was.is_none_or(|w| w[i] != n))
            .map(|(_, (n, (_, cls)))| (cls, n))
            .collect(),
    }
}

/// Where the tip's dashed line ends on each screen, see [`Cell::reach`]. The wide screen
/// hides `xw`, so its edge line is `ow`, shown only there; the narrow one's is `on`.
fn tip_ends(col: u16) -> Vec<(u32, &'static str)> {
    let mut out = vec![(x(col), hide(col))];
    for (keep, cls) in FOLDS {
        if col >= keep {
            out.push((u32::from(keep) * COL, if cls == "n" { "on" } else { "ow" }));
        }
    }
    out
}

/// Columns of `p` each fold hides, in [`FOLDS`] order.
fn hidden(p: &Placed) -> [u16; 2] {
    FOLDS.map(|(keep, _)| (reach(p) + 1).saturating_sub(keep))
}

/// The classes of the folds that hide something drawn in column `col`: the wide cap is past
/// the narrow one, so what the wide screen hides the narrow one hides too.
const fn hide(col: u16) -> &'static str {
    match col {
        c if c >= FOLDS[1].0 => "xn xw",
        c if c >= FOLDS[0].0 => "xn",
        _ => "",
    }
}

fn x(col: u16) -> u32 {
    u32::from(col) * COL + COL / 2
}

/// One half-row piece of line, from `y0` down to `y0 + H / 2`: straight, or a curve between
/// two columns.
fn line(seg: Seg, y0: u32) -> Line {
    let cls = line_class(seg);
    let (a, b, y1) = (x(seg.from), x(seg.to), y0 + H / 2);
    let m = y0 + H / 4;
    let d = if a == b {
        format!("M{a} {y0}V{y1}")
    } else {
        format!("M{a} {y0}C{a} {m} {b} {m} {b} {y1}")
    };
    Line { d, cls, k: lane(seg.color) }
}

/// A piece of line's classes: `main` on the first-parent chain, and the folds that hide it.
const fn line_class(seg: Seg) -> &'static str {
    let far = if seg.from > seg.to { seg.from } else { seg.to };
    match (seg.color == 0, far >= FOLDS[1].0, far >= FOLDS[0].0) {
        (true, true, _) => "main xn xw",
        (true, false, true) => "main xn",
        (true, false, false) => "main",
        (false, ..) => hide(far),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(rows: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        rows.iter()
            .map(|(id, ps)| ((*id).to_owned(), ps.iter().map(|p| (*p).to_owned()).collect()))
            .collect()
    }

    #[test]
    fn a_merged_branch_curves_out_and_back_into_the_mainline() {
        let hist = h(&[("m", &["a", "b"]), ("b", &["a"]), ("a", &[])]);
        let rows = place(&hist, 0, 3);
        assert_eq!(rows.iter().map(|p| p.col).collect::<Vec<_>>(), [0, 1, 0]);
        let merge = cell(&rows[0], None, columns(&rows), true).to_string();
        assert!(merge.contains('C'), "the merge opens a curve: {merge}");
        assert!(merge.contains("class=\"m main k0\""), "{merge}");
        assert!(merge.contains("aria-hidden=\"true\""));
    }

    #[test]
    fn a_later_page_carries_the_lines_from_the_page_above() {
        let hist = h(&[("m", &["a", "b"]), ("b", &["a"]), ("a", &[])]);
        let whole = place(&hist, 0, 3);
        assert_eq!(place(&hist, 1, 3), whole[1..]);
    }

    #[test]
    fn a_parent_above_its_child_or_outside_the_history_is_a_stub() {
        let hist = h(&[("b", &["x"]), ("c", &["b", "zz"]), ("x", &[])]);
        let rows = place(&hist, 0, 3);
        assert!(rows[1].stubs >= 1, "{rows:?}");
        assert!(
            cell(&rows[1], Some(&rows[0]), columns(&rows), true)
                .to_string()
                .contains("class=\"stub\"")
        );
    }

    #[test]
    fn columns_past_three_and_eight_fold_into_a_count() {
        let tips: Vec<String> = (0..10).map(|i| format!("t{i}")).collect();
        let mut hist: Vec<(String, Vec<String>)> = vec![("m".to_owned(), tips.clone())];
        for t in &tips {
            hist.push((t.clone(), vec!["root".to_owned()]));
        }
        hist.push(("root".to_owned(), vec!["base".to_owned()]));
        hist.push(("base".to_owned(), Vec::new()));
        let rows = place(&hist, 0, hist.len());
        let c = cell(&rows[0], None, columns(&rows), true).to_string();
        // Ten parents, ten columns: 7 past the narrow cap, 2 past the wide one.
        assert!(c.contains("class=\"gfold n\">+7<"), "{c}");
        assert!(c.contains("class=\"gfold w\">+2<"), "{c}");
        // Below the point where the lines join, the mainline alone needs no fold.
        let last = cell(&rows[hist.len() - 1], None, columns(&rows), false);
        assert_eq!(last.folds.len(), 0);
    }

    #[test]
    fn a_line_past_a_cap_is_not_drawn_and_its_count_shows_once_per_run() {
        let tips: Vec<String> = (0..15).map(|i| format!("t{i}")).collect();
        let mut hist: Vec<(String, Vec<String>)> = vec![("m".to_owned(), tips.clone())];
        for t in &tips {
            hist.push((t.clone(), vec!["root".to_owned()]));
        }
        hist.push(("root".to_owned(), Vec::new()));
        let rows = place(&hist, 0, hist.len());
        let w = columns(&rows);
        for (i, p) in rows.iter().enumerate() {
            let c = cell(p, i.checked_sub(1).map(|j| &rows[j]), w, false);
            for l in &c.paths {
                // x is every even number of "MxyVy" and "MxyCxyxyxy".
                let nums: Vec<u32> =
                    l.d.split(|ch: char| !ch.is_ascii_digit())
                        .filter_map(|n| n.parse().ok())
                        .collect();
                let reach_x = nums.iter().step_by(2).take(if l.d.contains('C') { 4 } else { 1 });
                let reach_x = reach_x.copied().max().unwrap_or(0);
                let cap = |k: u16| u32::from(k) * COL;
                assert!(reach_x < cap(3) || l.cls.contains("xn"), "row {i}: {l:?}");
                assert!(reach_x < cap(8) || l.cls.contains("xw"), "row {i}: {l:?}");
            }
            // Every stroke of the cell, the tip's dashed line too, ends inside each fold.
            for (end, cls) in &c.reach {
                let cap = |k: u16| u32::from(k) * COL;
                let on_narrow = !cls.contains("xn") && *cls != "ow";
                let on_wide = !cls.contains("xw") && *cls != "on";
                assert!(!on_narrow || *end <= cap(3), "row {i}: tip to {end} {cls}");
                assert!(!on_wide || *end <= cap(8), "row {i}: tip to {end} {cls}");
            }
            assert_eq!(c.edge.is_empty(), p.col < 3, "row {i}: {:?}", c.edge);
            if i > 1 && hidden(p) == hidden(&rows[i - 1]) {
                assert!(c.folds.is_empty(), "row {i} repeats its count: {:?}", c.folds);
            }
        }
    }
}
