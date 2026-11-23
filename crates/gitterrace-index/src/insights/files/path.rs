//! The path card: what a file or directory card shows, read from the reports the pages
//! read, narrowed to one path.
//!
//! Nothing here counts on its own. The churn rank is the row of [`Insights::hotspots_in`],
//! the owners are [`Insights::ownership_in`] over the path's files, the partners are the
//! pairs of [`Insights::coupling_in`] that name the path. A card therefore cannot drift from
//! the full report: the oracle test holds the two equal. The touches of the path itself
//! (sparkline, last change, recent authors) come from one `visit_touches_in`, which reads a
//! file's postings when the path set is small and the shared tip index above that bound.

use std::collections::{BTreeMap, HashSet};

use rustc_hash::FxHashMap as FastMap;

use crate::feed::local_day_of;
use crate::insights::files::Owner;
use crate::insights::{Counted, DAY, HeadFile, Insights};

/// Owners a card lists.
pub const CARD_OWNERS: usize = 3;
/// Partners a card lists.
pub const CARD_PARTNERS: usize = 3;
/// Buckets of the sparkline.
pub const CARD_BUCKETS: usize = 12;
/// Paths per touch visit of a directory card. Above the postings lookup's bound a visit
/// reads the shared tip index, built once per set: 64 paths per visit looked each path up
/// in postings and measured 11 times slower on a 1.1k-file directory. A bound, not a
/// setting: the answer is the same for any size.
const TOUCH_CHUNK: usize = 256;
/// Days before "now" that count as recent for the authors figure.
pub const RECENT_DAYS: i64 = 90;
/// Shared commits a pair needs to be a partner: the patterns page's own bar.
pub const PARTNER_MIN_TOGETHER: usize = 3;

/// What a path is at the tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum PathKind {
    File,
    Dir,
    /// Not a text file or a directory of the head tree.
    Missing,
}

/// A file that changes together with the card's file.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Partner {
    pub path: String,
    pub together: usize,
    /// `together / min(changes of the two)`, as [`crate::Coupling::degree`].
    pub degree: f64,
}

/// A direct child of a directory card, enough for a tree badge. A child directory reads as
/// its files together: changes summed, the best rank among them, hot when any file is.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PathChild {
    pub path: String,
    pub hot: bool,
    pub changes: usize,
    /// 1-based by hotspot score, 0 when no file under it has a counted change.
    pub rank: usize,
}

/// The card of one path, see the module docs.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PathCard {
    pub kind: PathKind,
    pub changes: usize,
    pub lines_changed: u64,
    /// 1-based by hotspot score (a directory: its best file), 0 when not ranked.
    pub rank: usize,
    /// Ranked files, 0 when `rank` is 0.
    pub of: usize,
    /// Rank within the top tenth: [`crate::HotspotReport::top_tenth`], the one definition.
    pub hot: bool,
    /// Changes per bucket over the period's span, oldest first.
    pub series: Vec<u32>,
    pub last_change: Option<i64>,
    /// People with a counted change in the last [`RECENT_DAYS`] days.
    pub authors_90d: usize,
    pub owners: Vec<Owner>,
    pub bus_factor: usize,
    /// Files only.
    pub partners: Vec<Partner>,
    /// Directories only, by path.
    pub children: Vec<PathChild>,
}

impl PathCard {
    fn missing() -> Self {
        Self {
            kind: PathKind::Missing,
            changes: 0,
            lines_changed: 0,
            rank: 0,
            of: 0,
            hot: false,
            series: vec![0; CARD_BUCKETS],
            last_change: None,
            authors_90d: 0,
            owners: Vec::new(),
            bus_factor: 0,
            partners: Vec::new(),
            children: Vec::new(),
        }
    }
}

impl Insights<'_> {
    /// The card of `path` (a file or a directory of `files`, the head tree) over the commits
    /// `c` counts. `""` is the root directory.
    pub fn path_card(&self, files: &[HeadFile], c: &Counted<'_>, path: &str) -> PathCard {
        self.path_card_chunked(files, c, path, TOUCH_CHUNK)
    }

    /// [`Insights::path_card`] reading the touches of a directory's files `chunk` paths at a
    /// time. The answer does not depend on `chunk`: it is a plan, which the latency test
    /// measures. `0` reads them in one visit.
    #[doc(hidden)]
    pub fn path_card_chunked(
        &self,
        files: &[HeadFile],
        c: &Counted<'_>,
        path: &str,
        chunk: usize,
    ) -> PathCard {
        let path = path.trim_matches('/');
        let is_file = !path.is_empty() && files.iter().any(|f| f.path == path);
        let prefix = if path.is_empty() { String::new() } else { format!("{path}/") };
        let members: Vec<&HeadFile> = files
            .iter()
            .filter(|f| if is_file { f.path == path } else { f.path.starts_with(&prefix) })
            .collect();
        if members.is_empty() {
            return PathCard::missing();
        }
        let report = self.hotspots_in(files, c);
        let at: FastMap<&str, usize> =
            report.files.iter().enumerate().map(|(i, h)| (&*h.path, i)).collect();
        let ranked = |f: &HeadFile| at.get(&*f.path).map(|&i| (i + 1, &report.files[i]));
        let (mut changes, mut lines_changed, mut rank) = (0, 0, 0);
        for (r, h) in members.iter().filter_map(|f| ranked(f)) {
            changes += h.changes;
            lines_changed += h.lines_changed;
            rank = best(rank, r);
        }
        let hot = |rank: usize| rank != 0 && rank <= report.top_tenth;
        // The files the reports count: the path selection and the skip rules apply to a card
        // as they do to a page.
        let matcher = c.scope().matcher();
        let counted: Vec<HeadFile> = members
            .iter()
            .filter(|f| f.lines != 0 && matcher.row(&f.path) && !self.skips_head(&f.path))
            .map(|f| (*f).clone())
            .collect();
        let names: Vec<&str> = counted.iter().map(|f| &*f.path).collect();
        let (series, last_change, authors_90d) = self.card_touches(&names, c, chunk);
        let (owners, bus_factor) = if counted.is_empty() {
            (Vec::new(), 0)
        } else {
            let own = self.ownership_in(&counted, c, 1).overall;
            (own.owners.into_iter().take(CARD_OWNERS).collect(), own.bus_factor)
        };
        let partners = if is_file && !counted.is_empty() {
            self.partners_of(files, c, path)
        } else {
            Vec::new()
        };
        let mut kids: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        if !is_file {
            for f in &members {
                let rest = f.path.get(prefix.len()..).unwrap_or_default();
                let seg = rest.split('/').next().unwrap_or_default();
                let e = kids.entry(format!("{prefix}{seg}")).or_default();
                if let Some((r, h)) = ranked(f) {
                    e.0 += h.changes;
                    e.1 = best(e.1, r);
                }
            }
        }
        PathCard {
            kind: if is_file { PathKind::File } else { PathKind::Dir },
            changes,
            lines_changed,
            rank,
            of: if rank == 0 { 0 } else { report.files.len() },
            hot: hot(rank),
            series,
            last_change,
            authors_90d,
            owners,
            bus_factor,
            partners,
            children: kids
                .into_iter()
                .map(|(path, (changes, rank))| PathChild { path, hot: hot(rank), changes, rank })
                .collect(),
        }
    }

    /// The pairs of the full coupling report that name `path`, strongest first.
    fn partners_of(&self, files: &[HeadFile], c: &Counted<'_>, path: &str) -> Vec<Partner> {
        self.coupling_in(files, c, PARTNER_MIN_TOGETHER, usize::MAX)
            .into_iter()
            .filter_map(|p| {
                let other = if p.a == path {
                    p.b
                } else if p.b == path {
                    p.a
                } else {
                    return None;
                };
                Some(Partner { path: other, together: p.together, degree: p.degree })
            })
            .take(CARD_PARTNERS)
            .collect()
    }

    /// The sparkline, the newest change and the recent authors of `names`, in one visit of
    /// their touches by the commits `c` counts.
    fn card_touches(
        &self,
        names: &[&str],
        c: &Counted<'_>,
        chunk: usize,
    ) -> (Vec<u32>, Option<i64>, usize) {
        let cx = c.context();
        let lo = (*c.days().start()).max(cx.first_day());
        let hi = (*c.days().end()).min(cx.last_day());
        let span = hi.saturating_sub(lo).saturating_add(1);
        let recent = cx.now().saturating_sub(RECENT_DAYS * DAY);
        let mut series = vec![0u32; CARD_BUCKETS];
        let mut last: Option<i64> = None;
        let mut who: HashSet<usize> = HashSet::new();
        // A directory's files are visited `chunk` at a time, so no visit holds more paths
        // than that.
        let size = if chunk == 0 { names.len().max(1) } else { chunk };
        for part in names.chunks(size) {
            self.visit_touches_in(part, c, |_, _, person, row| {
                let day = local_day_of(row.time, row.offset);
                if span > 0 && (lo..=hi).contains(&day) {
                    let slot = (day - lo).saturating_mul(CARD_BUCKETS as i64) / span;
                    let slot = usize::try_from(slot).unwrap_or(0).min(CARD_BUCKETS - 1);
                    series[slot] += 1;
                }
                last = last.max(Some(row.time));
                if row.time >= recent {
                    who.insert(person);
                }
                true
            });
        }
        (series, last, who.len())
    }
}

/// The better of two ranks, 0 meaning none.
const fn best(a: usize, b: usize) -> usize {
    if a == 0 || (b != 0 && b < a) { b } else { a }
}
