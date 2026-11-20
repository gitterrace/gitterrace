//! Columns of the commit graph: the column of each commit and of each line between commits.
//!
//! The input is commits newest first in topological order, each with its parents. An open column
//! waits for one commit. The rule is git's own greedy one: a commit takes the column
//! waiting for it (the leftmost, when several wait), a branch tip takes the leftmost free column
//! right of column 0, and the column then waits for the commit's first parent, so a line keeps its
//! column for as long as it lives. A merge's further parents join a column already waiting for
//! them or open the leftmost free one. Column 0 is the first-parent chain of the first commit
//! laid out, and nothing else ever takes it.
//!
//! The layout is a fold over rows. A page starts from the [`Open`] the page above ended with,
//! so the lines leaving the bottom of one page are the lines entering the top of the next,
//! column for column and colour for colour.

/// Rows between a commit and its parent past which the edge is a stub, not a line.
///
/// Not configurable. Measured in topological order: with every edge a line, git, rust and
/// linux keep 125, 141 and 299 lines open at the median row. With edges over 200 rows stubbed,
/// the 99th percentile is 33, 32 and 11. The first-parent chain in column 0 is never stubbed.
pub const FAR_ROWS: u32 = 200;

/// One parent of a row. `far` marks an edge the caller wants drawn as a stub: its parent lies
/// more than [`FAR_ROWS`] below, or outside the commits laid out at all (a shallow boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parent {
    pub id: u32,
    pub far: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub parents: Vec<Parent>,
}

/// A piece of line inside one row, from column `from` to column `to`. In [`Placed::top`] it runs
/// from the row's top edge to its middle, in [`Placed::bottom`] from the middle to the bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Seg {
    pub from: u16,
    pub to: u16,
    /// The line's colour, 0 for column 0, then numbered as lines open.
    pub color: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Placed {
    pub col: u16,
    pub color: u32,
    pub tip: bool,
    pub top: Vec<Seg>,
    pub bottom: Vec<Seg>,
    pub stubs: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Line {
    waits: u32,
    color: u32,
}

/// The lines open between two rows: what a page needs from the page above it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Open {
    cols: Vec<Option<Line>>,
    colors: u32,
    started: bool,
}

impl Open {
    /// Columns in use, the rightmost open column plus one.
    pub const fn width(&self) -> u16 {
        self.cols.len() as u16
    }

    pub fn place(&mut self, row: &Row) -> Placed {
        let first = !self.started;
        self.started = true;
        let hits: Vec<usize> = self.waiting_for(row.id).collect();
        let col = match hits.first() {
            Some(&c) => c,
            None if first => 0,
            None => self.free_from(1),
        };
        self.reach(col);
        let color = match hits.first() {
            Some(&c) => self.cols[c].map_or(0, |l| l.color),
            None if col == 0 => 0,
            None => self.fresh(),
        };
        let top = self
            .lines()
            .map(|(i, l)| Seg {
                from: i as u16,
                to: if l.waits == row.id { col as u16 } else { i as u16 },
                color: l.color,
            })
            .collect();
        for &h in &hits {
            self.cols[h] = None;
        }
        let mut stubs = 0u16;
        let mut opened = vec![col];
        let mut joins = Vec::new();
        if let Some(p) = row.parents.first() {
            if p.far && col != 0 {
                stubs += 1;
            } else {
                self.cols[col] = Some(Line { waits: p.id, color });
            }
        }
        for p in row.parents.iter().skip(1) {
            if p.far {
                stubs += 1;
            } else if let Some(j) = self.waiting_for(p.id).next() {
                if j != col {
                    joins.push(j);
                }
            } else {
                let j = self.free_from(1);
                self.reach(j);
                let c = self.fresh();
                self.cols[j] = Some(Line { waits: p.id, color: c });
                opened.push(j);
            }
        }
        let mut bottom: Vec<Seg> = self
            .lines()
            .map(|(i, l)| Seg {
                from: if opened.contains(&i) { col as u16 } else { i as u16 },
                to: i as u16,
                color: l.color,
            })
            .collect();
        for j in joins {
            if let Some(l) = self.cols[j] {
                bottom.push(Seg { from: col as u16, to: j as u16, color: l.color });
            }
        }
        while self.cols.last().is_some_and(Option::is_none) {
            self.cols.pop();
        }
        Placed { col: col as u16, color, tip: hits.is_empty(), top, bottom, stubs }
    }

    fn lines(&self) -> impl Iterator<Item = (usize, Line)> + '_ {
        self.cols.iter().enumerate().filter_map(|(i, l)| l.map(|l| (i, l)))
    }

    fn waiting_for(&self, id: u32) -> impl Iterator<Item = usize> + '_ {
        self.lines().filter(move |(_, l)| l.waits == id).map(|(i, _)| i)
    }

    fn free_from(&self, start: usize) -> usize {
        (start..self.cols.len())
            .find(|&i| self.cols[i].is_none())
            .unwrap_or_else(|| self.cols.len().max(start))
    }

    fn reach(&mut self, col: usize) {
        if self.cols.len() <= col {
            self.cols.resize(col + 1, None);
        }
    }

    const fn fresh(&mut self) -> u32 {
        self.colors += 1;
        self.colors
    }
}

/// Lays out `rows` below `open`, leaving `open` as the state after the last row.
#[cfg(test)]
fn place_all(open: &mut Open, rows: &[Row]) -> Vec<Placed> {
    rows.iter().map(|r| open.place(r)).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use proptest::prelude::*;

    use super::{Open, Parent, Placed, Row, place_all};

    fn row(id: u32, parents: &[u32]) -> Row {
        Row { id, parents: parents.iter().map(|&id| Parent { id, far: false }).collect() }
    }

    fn cols(placed: &[Placed]) -> Vec<u16> {
        placed.iter().map(|p| p.col).collect()
    }

    /// Every property the layout promises, over rows in topological order (each parent below
    /// its child). Returns the first broken one.
    fn check(rows: &[Row], placed: &[Placed]) -> Result<(), String> {
        let pos: HashMap<u32, usize> = rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        let mut chain = Vec::new();
        let mut at = rows.first().map(|r| r.id);
        while let Some(id) = at {
            chain.push(id);
            at = pos.get(&id).and_then(|&i| rows[i].parents.first()).map(|p| p.id);
        }
        let mut open = Open::default();
        for (r, (row, want)) in rows.iter().zip(placed).enumerate() {
            let before = open.clone();
            let got = open.place(row);
            if &got != want {
                return Err(format!("row {r}: paged layout differs from one pass"));
            }
            let col = got.col as usize;
            // A dot sits only on a column that is free or waits for it: no line crosses a node.
            if before.cols.get(col).copied().flatten().is_some_and(|l| l.waits != row.id) {
                return Err(format!("row {r}: column {col} passes through the commit"));
            }
            if (col == 0) != chain.contains(&row.id) {
                return Err(format!("row {r}: column 0 is not the first-parent chain"));
            }
            for (i, l) in open.lines() {
                if pos.get(&l.waits).is_none_or(|&p| p <= r) {
                    return Err(format!("row {r}: column {i} waits for a commit not below"));
                }
                // A line keeps its colour: one open before and not restarted here is unchanged.
                if i != col
                    && let Some(b) = before.cols.get(i).copied().flatten()
                    && b.waits == l.waits
                    && b.color != l.color
                {
                    return Err(format!("row {r}: column {i} changed colour"));
                }
            }
            for (k, p) in row.parents.iter().enumerate() {
                let kept = !p.far || (k == 0 && col == 0);
                if kept && open.waiting_for(p.id).next().is_none() {
                    return Err(format!("row {r}: parent {} has no column", p.id));
                }
            }
            if !got.tip && got.color != before.cols[col].map_or(u32::MAX, |l| l.color) {
                return Err(format!("row {r}: the commit left its line's colour"));
            }
            // The lines leaving a row are the lines entering the next one.
            if let Some(next) = placed.get(r + 1) {
                let mut down: Vec<(u16, u32)> =
                    got.bottom.iter().map(|s| (s.to, s.color)).collect();
                down.sort_unstable();
                down.dedup();
                let mut up: Vec<(u16, u32)> = next.top.iter().map(|s| (s.from, s.color)).collect();
                up.sort_unstable();
                if down != up {
                    return Err(format!("row {r}: lines do not join the next row"));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn a_line_of_commits_stays_in_column_zero() {
        let rows = [row(1, &[2]), row(2, &[3]), row(3, &[])];
        let placed = place_all(&mut Open::default(), &rows);
        assert_eq!(cols(&placed), [0, 0, 0]);
        assert!(placed.iter().all(|p| p.color == 0));
        check(&rows, &placed).unwrap();
    }

    #[test]
    fn a_merged_branch_takes_column_one_and_ends_at_its_fork() {
        // 1 merges 3 into 2. Both fork from 4.
        let rows = [row(1, &[2, 3]), row(3, &[4]), row(2, &[4]), row(4, &[])];
        let placed = place_all(&mut Open::default(), &rows);
        assert_eq!(cols(&placed), [0, 1, 0, 0]);
        assert_eq!(placed[1].color, 1);
        assert!(!placed[1].tip);
        // At the fork the side line curves back into column 0.
        assert!(placed[3].top.iter().any(|s| s.from == 1 && s.to == 0));
        check(&rows, &placed).unwrap();
    }

    #[test]
    fn an_octopus_opens_a_column_per_further_parent() {
        let rows = [
            row(1, &[2, 3, 4, 5]),
            row(5, &[6]),
            row(4, &[6]),
            row(3, &[6]),
            row(2, &[6]),
            row(6, &[]),
        ];
        let placed = place_all(&mut Open::default(), &rows);
        assert_eq!(cols(&placed), [0, 3, 2, 1, 0, 0]);
        assert_eq!(placed[0].bottom.len(), 4);
        check(&rows, &placed).unwrap();
    }

    #[test]
    fn a_far_parent_is_a_stub_and_frees_its_column() {
        let mut rows = [row(1, &[2, 3]), row(3, &[4]), row(2, &[4]), row(4, &[])];
        rows[1].parents[0].far = true;
        let placed = place_all(&mut Open::default(), &rows);
        assert_eq!(placed[1].stubs, 1);
        assert!(placed[1].bottom.iter().all(|s| s.to == 0));
        check(&rows, &placed).unwrap();
    }

    #[test]
    fn column_zero_never_stubs_its_first_parent() {
        let rows = [Row { id: 1, parents: vec![Parent { id: 2, far: true }] }, row(2, &[])];
        let placed = place_all(&mut Open::default(), &rows);
        assert_eq!(placed[0].stubs, 0);
        assert_eq!(cols(&placed), [0, 0]);
    }

    #[test]
    fn a_new_tip_never_takes_column_zero() {
        // Two roots: after the first chain ends, the second history starts beside it.
        let rows = [row(1, &[]), row(2, &[3]), row(3, &[])];
        let placed = place_all(&mut Open::default(), &rows);
        assert_eq!(cols(&placed), [0, 1, 1]);
        assert!(placed[1].tip);
    }

    /// A random history in topological order, newest first: each commit's parents lie below it.
    /// Edges longer than `far` rows are stubs, as the caller marks them.
    fn history() -> impl Strategy<Value = (Vec<Row>, Vec<usize>)> {
        (2usize..80, 1u32..30).prop_flat_map(|(n, far)| {
            let picks = proptest::collection::vec(
                (0usize..10, proptest::collection::vec(any::<prop::sample::Index>(), 8)),
                n,
            );
            let cuts = proptest::collection::vec(1usize..12, 1..10);
            (picks, cuts).prop_map(move |(picks, cuts)| {
                let rows = picks
                    .iter()
                    .enumerate()
                    .map(|(i, (kind, idx))| {
                        let below = n - i - 1;
                        // Mostly one parent, some merges, a few octopus merges and roots.
                        let count = match kind {
                            _ if below == 0 => 0,
                            0 => 0,
                            1 | 2 => 2,
                            3 => idx.len(),
                            _ => 1,
                        };
                        let parents = idx[..count]
                            .iter()
                            .enumerate()
                            .map(|(k, x)| {
                                // The first parent is usually close, as a branch's own history is.
                                let span =
                                    if k == 0 { x.index(below.min(3)) } else { x.index(below) };
                                let j = i + 1 + span;
                                Parent { id: j as u32, far: (j - i) as u32 > far }
                            })
                            .collect();
                        Row { id: i as u32, parents }
                    })
                    .collect();
                (rows, cuts)
            })
        })
    }

    proptest! {
        #[test]
        fn layout_holds_its_properties_across_any_paging((rows, cuts) in history()) {
            let mut open = Open::default();
            let mut placed = Vec::new();
            let mut at = 0;
            for size in cuts.iter().cycle() {
                if at >= rows.len() {
                    break;
                }
                let end = (at + size).min(rows.len());
                // A page resumes from a copy of the state, as a checkpoint is.
                let mut page = open.clone();
                placed.extend(place_all(&mut page, &rows[at..end]));
                open = page;
                at = end;
            }
            prop_assert_eq!(check(&rows, &placed), Ok(()));
        }
    }
}
