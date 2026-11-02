//! The option matrix of the equivalence guard, generated as a product so no cell is picked by
//! hand: period x bots x mailmap x path filter x whitespace x selection chip.

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
        match self.chip {
            Chip::None => {}
            Chip::Author => q.push(format!("author={}", enc(&v.author))),
            Chip::Kind => q.push(format!("type={}", enc(&v.kind))),
            Chip::Scope => q.push(format!("scope={}", enc(&v.scope))),
            Chip::Path => q.push(format!("path={}", enc(&v.path))),
        }
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

    use crate::matrix::{Chip, Mailmap, PathFilter, cells, sample};

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
}
