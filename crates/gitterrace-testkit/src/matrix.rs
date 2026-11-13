//! The option matrix of the equivalence guard.
//!
//! The axes are period x bots x mailmap x path filter x whitespace x selection chip. [`cells`] is
//! the full product (the per-commit proof and the revisions golden), [`pairwise`] a generated
//! covering array of it (the equivalence golden), and [`named`] the high-risk cells real history
//! broke, each with its reason.

/// Every period value the filter bar offers.
pub const PERIODS: [&str; 5] = ["1w", "1m", "3m", "1y", "all"];

/// Which `.mailmap` the repository carries: as built, or edited by a later commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mailmap {
    AsIs,
    Edited,
    /// Edited, plus a case-only different e-mail folded by the mailmap.
    CaseFold,
}

/// The path filter options: none, one exclude glob, generated files on, ignore rules off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PathFilter {
    None,
    Exclude,
    Generated,
    IgnoreOff,
}

/// One selection chip, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Chip {
    None,
    Author,
    Kind,
    Scope,
    Path,
}

/// The values a repository offers for each chip and for the exclude glob.
#[derive(Debug, Clone)]
pub struct Values {
    pub author: String,
    pub kind: String,
    pub scope: String,
    pub path: String,
    pub exclude: String,
}

/// One cell of the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cell {
    pub period: &'static str,
    pub bots: bool,
    pub mailmap: Mailmap,
    pub filter: PathFilter,
    pub ws: bool,
    pub chip: Chip,
}

impl Cell {
    /// The query string for this cell, without `?`. The mailmap axis is not a parameter: it
    /// picks the repository the cell runs against.
    #[must_use]
    pub fn query(&self, v: &Values) -> String {
        let mut q = vec![format!("period={}", self.period)];
        q.push(format!("bots={}", u8::from(self.bots)));
        match self.filter {
            PathFilter::None => {}
            PathFilter::Exclude => q.push(format!("exclude={}", enc(&v.exclude))),
            PathFilter::Generated => q.push("generated=1".to_owned()),
            PathFilter::IgnoreOff => q.push("ignore=0".to_owned()),
        }
        if self.ws {
            q.push("ws=1".to_owned());
        }
        let term = match self.chip {
            Chip::None => None,
            Chip::Author => Some(format!("author:{}", v.author)),
            Chip::Kind => Some(format!("type:{}", v.kind)),
            Chip::Scope => Some(format!("scope:{}", v.scope)),
            Chip::Path => Some(format!("path:{}", v.path)),
        };
        q.extend(term.map(|t| format!("q={}", enc(&t))));
        q.join("&")
    }
}

/// The full product, in a fixed order: 5 x 2 x 3 x 4 x 2 x 5 = 1200 cells.
#[must_use]
pub fn cells() -> Vec<Cell> {
    let mut out = Vec::new();
    for period in PERIODS {
        for bots in [false, true] {
            for mailmap in [Mailmap::AsIs, Mailmap::Edited, Mailmap::CaseFold] {
                for filter in [
                    PathFilter::None,
                    PathFilter::Exclude,
                    PathFilter::Generated,
                    PathFilter::IgnoreOff,
                ] {
                    for ws in [false, true] {
                        for chip in [
                            Chip::None,
                            Chip::Author,
                            Chip::Kind,
                            Chip::Scope,
                            Chip::Path,
                        ] {
                            out.push(Cell {
                                period,
                                bots,
                                mailmap,
                                filter,
                                ws,
                                chip,
                            });
                        }
                    }
                }
            }
        }
    }
    out
}

/// The value count of each axis, in [`Cell`] field order.
pub const AXES: [usize; 6] = [5, 2, 3, 4, 2, 5];

const MAILMAPS: [Mailmap; 3] = [Mailmap::AsIs, Mailmap::Edited, Mailmap::CaseFold];
const FILTERS: [PathFilter; 4] = [
    PathFilter::None,
    PathFilter::Exclude,
    PathFilter::Generated,
    PathFilter::IgnoreOff,
];
const CHIPS: [Chip; 5] = [
    Chip::None,
    Chip::Author,
    Chip::Kind,
    Chip::Scope,
    Chip::Path,
];

impl Cell {
    /// The value index of each axis, in [`AXES`] order.
    #[must_use]
    pub fn axes(&self) -> [usize; 6] {
        let at = |found: Option<usize>| found.unwrap_or(0);
        [
            at(PERIODS.iter().position(|p| *p == self.period)),
            usize::from(self.bots),
            at(MAILMAPS.iter().position(|m| *m == self.mailmap)),
            at(FILTERS.iter().position(|f| *f == self.filter)),
            usize::from(self.ws),
            at(CHIPS.iter().position(|c| *c == self.chip)),
        ]
    }

    /// The cell with these axis values.
    #[must_use]
    pub const fn of(
        period: &'static str,
        bots: bool,
        mailmap: Mailmap,
        filter: PathFilter,
        ws: bool,
        chip: Chip,
    ) -> Self {
        Self {
            period,
            bots,
            mailmap,
            filter,
            ws,
            chip,
        }
    }
}

/// One value pair a covering array must hold: axes `a < b` with values `va`, `vb`.
pub type Pair = (usize, usize, usize, usize);

/// Every value pair of two different axes `cell` holds.
#[must_use]
pub fn pairs_of(cell: &Cell) -> Vec<Pair> {
    let v = cell.axes();
    let mut out = Vec::new();
    for a in 0..v.len() {
        for b in a + 1..v.len() {
            out.push((a, v[a], b, v[b]));
        }
    }
    out
}

/// Every value pair of every two axes: what [`pairwise`] must cover.
#[must_use]
pub fn all_pairs() -> std::collections::BTreeSet<Pair> {
    let mut out = std::collections::BTreeSet::new();
    for (a, &na) in AXES.iter().enumerate() {
        for (b, &nb) in AXES.iter().enumerate().skip(a + 1) {
            for va in 0..na {
                for vb in 0..nb {
                    out.insert((a, va, b, vb));
                }
            }
        }
    }
    out
}

/// A pairwise (2-wise) covering array of [`cells`].
///
/// Every value of every axis meets every value of every other axis in at least one cell. Greedy and
/// deterministic: each round takes the first product cell that covers the most uncovered pairs,
/// then any cell whose pairs all others hold is dropped, so removing any one cell uncovers a pair.
/// Sorted.
#[must_use]
pub fn pairwise() -> Vec<Cell> {
    let all = cells();
    let mut open = all_pairs();
    let mut picked: Vec<Cell> = Vec::new();
    while !open.is_empty() {
        let mut best: Option<(usize, &Cell)> = None;
        for c in &all {
            let n = pairs_of(c).iter().filter(|p| open.contains(p)).count();
            if best.is_none_or(|(m, _)| n > m) {
                best = Some((n, c));
            }
        }
        let Some((_, c)) = best else { break };
        for p in pairs_of(c) {
            open.remove(&p);
        }
        picked.push(*c);
    }
    let mut at = 0;
    while at < picked.len() {
        let rest: std::collections::BTreeSet<Pair> = picked
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != at)
            .flat_map(|(_, c)| pairs_of(c))
            .collect();
        if pairs_of(&picked[at]).iter().all(|p| rest.contains(p)) {
            picked.remove(at);
        } else {
            at += 1;
        }
    }
    picked.sort();
    picked
}

/// A cell the pairwise array may miss but real history broke: asked on `page` only.
#[derive(Debug, Clone, Copy)]
pub struct Named {
    pub page: &'static str,
    pub cell: Cell,
    pub why: &'static str,
}

/// The named high-risk cells, in a fixed order.
///
/// The non-HEAD revisions are not here: their
/// golden (`revisions.txt`) keeps all its cells, since each revision loads its own record set
/// and a pairwise cut would leave most rev x option pairs to one cell.
#[must_use]
pub fn named() -> Vec<Named> {
    use Chip as C;
    use Mailmap as M;
    use PathFilter as F;
    const TIES: &str = "equal counts at equal times: the segment rewrite once reordered 144 \
                        commit types cells";
    const OWN: &str = "ownership over all time follows paths across their whole history, \
                       where a renamed and reused path would collide";
    const MAP: &str = "an edited or case-folded mailmap with bots off: identities merge after \
                       the fact and must count once";
    const FILES: &str = "exclude glob plus whitespace-blind lines on the file pages: two \
                         path-level filters on one walk";
    const SIDE: &str = "the overview sidebar, cut by the chrome check and re-blessed twice \
                        after sidebar changes";
    let mut out = Vec::new();
    let mut add = |pages: &[&'static str], cells: &[Cell], why: &'static str| {
        for &page in pages {
            for cell in cells {
                out.push(Named {
                    page,
                    cell: *cell,
                    why,
                });
            }
        }
    };
    let ties: Vec<Cell> = PERIODS
        .into_iter()
        .flat_map(|p| {
            [C::None, C::Kind, C::Scope].map(|c| Cell::of(p, false, M::AsIs, F::None, false, c))
        })
        .collect();
    add(&["/stats", "/api/v1/insights/label_breakdown"], &ties, TIES);
    let own: Vec<Cell> = MAILMAPS
        .iter()
        .flat_map(|m| {
            [C::None, C::Path, C::Author].map(|c| Cell::of("all", false, *m, F::None, false, c))
        })
        .collect();
    add(
        &["/stats/ownership", "/api/v1/insights/ownership"],
        &own,
        OWN,
    );
    let map: Vec<Cell> = [M::Edited, M::CaseFold]
        .iter()
        .flat_map(|m| {
            ["1w", "1m", "all"].into_iter().flat_map(move |p| {
                [C::None, C::Author].map(|c| Cell::of(p, false, *m, F::None, false, c))
            })
        })
        .collect();
    add(
        &[
            "/stats/contributors",
            "/api/v1/contributors",
            "/api/v1/insights/contributors",
            "/api/v1/insights/ownership",
        ],
        &map,
        MAP,
    );
    let files: Vec<Cell> = PERIODS
        .into_iter()
        .flat_map(|p| [false, true].map(|b| Cell::of(p, b, M::AsIs, F::Exclude, true, C::None)))
        .collect();
    add(
        &[
            "/stats/hotspots",
            "/stats/languages",
            "/api/v1/insights/hotspots",
            "/api/v1/insights/file_churn",
            "/api/v1/insights/dir_churn",
        ],
        &files,
        FILES,
    );
    let side: Vec<Cell> = MAILMAPS
        .iter()
        .flat_map(|m| ["1w", "all"].map(|p| Cell::of(p, false, *m, F::None, false, C::None)))
        .collect();
    add(&["/"], &side, SIDE);
    out
}

/// `n` cells drawn from [`cells`] by a seeded generator, without repeats, in matrix order.
#[must_use]
pub fn sample(n: usize, seed: u64) -> Vec<Cell> {
    let mut all = cells();
    let mut rng = crate::synthetic::Rng::new(seed);
    let n = n.min(all.len());
    for i in 0..n {
        let j = i + usize::try_from(rng.below((all.len() - i) as u64)).unwrap_or(0);
        all.swap(i, j);
    }
    all.truncate(n);
    all.sort();
    all
}

/// Percent-encodes everything outside the unreserved set.
#[must_use]
pub fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::matrix::{
        Chip, Mailmap, PathFilter, all_pairs, cells, named, pairs_of, pairwise, sample,
    };

    /// The product really is complete: every axis value appears with every other.
    #[test]
    fn the_matrix_is_the_full_product() {
        let all = cells();
        assert_eq!(all.len(), 1200);
        assert_eq!(all.iter().collect::<BTreeSet<_>>().len(), 1200);
        for c in [
            Chip::None,
            Chip::Author,
            Chip::Kind,
            Chip::Scope,
            Chip::Path,
        ] {
            for f in [
                PathFilter::None,
                PathFilter::Exclude,
                PathFilter::Generated,
                PathFilter::IgnoreOff,
            ] {
                let n = all
                    .iter()
                    .filter(|x| x.chip == c && x.filter == f && x.mailmap == Mailmap::Edited)
                    .count();
                assert_eq!(n, 20, "{c:?} {f:?}");
            }
        }
        let s = sample(64, 9);
        assert_eq!(s.len(), 64);
        assert_eq!(s, sample(64, 9));
        assert_ne!(s, sample(64, 10));
    }

    /// Every value pair of every two axes is in some pairwise cell, and the array is minimal:
    /// dropping any one cell leaves a pair uncovered, so the check is red on a lost cell.
    #[test]
    fn the_pairwise_array_covers_every_pair() {
        let set = pairwise();
        let want = all_pairs();
        assert_eq!(want.len(), 179);
        let covered = |cs: &[crate::matrix::Cell]| {
            let have: BTreeSet<_> = cs.iter().flat_map(pairs_of).collect();
            want.iter().filter(|p| !have.contains(p)).count()
        };
        assert_eq!(covered(&set), 0);
        for i in 0..set.len() {
            let mut less = set.clone();
            less.remove(i);
            assert!(covered(&less) > 0, "cell {i} is redundant: {:?}", set[i]);
        }
        assert!(set.len() < 40, "{}", set.len());
    }

    /// The same array on every call and every machine: a pinned digest of its rendering.
    #[test]
    fn the_pairwise_array_is_deterministic() {
        let set = pairwise();
        assert_eq!(set, pairwise());
        let text = format!("{set:?}");
        assert_eq!(
            crate::oracle::digest(&text),
            PAIRWISE_DIGEST,
            "{} cells",
            set.len()
        );
    }

    const PAIRWISE_DIGEST: &str = "ded7cc9d8ad855cc57ac016016c9551e";

    /// Every named cell carries a reason and is a cell of the product.
    #[test]
    fn every_named_cell_has_a_reason() {
        let all: BTreeSet<_> = cells().into_iter().collect();
        for n in named() {
            assert!(n.why.len() > 20 && n.page.starts_with('/'), "{n:?}");
            assert!(all.contains(&n.cell), "{n:?}");
        }
    }
}
