//! The reducers that fold a [`Counted`] stream: totals, series, tallies and size buckets.

use std::collections::{BTreeSet, HashMap};

use crate::feed::{InsightsSource, LabelId, PathId};
use crate::insights::counted::{Counted, Item, Reducer, SIZE_EDGES, SIZE_LABELS};
use crate::insights::fasthash::{FastMap, FastSet};
use crate::insights::{
    Label, LabelBucket, LabelCount, PeriodStats, Selection, SeriesPoint, SizeBucket, TimeAxis,
    weekday,
};

/// Commits, authors, distinct counted files and lines.
#[derive(Default)]
pub struct Totals {
    s: PeriodStats,
    people: FastSet<usize>,
    files: FastSet<PathId>,
    /// The distinct files when a table already holds them: the set reads no rows then.
    known_files: Option<usize>,
}

impl Totals {
    /// Totals over `c`. The whole unselected history takes its file count from the shared
    /// table, so it streams commit columns only.
    pub fn of(c: &Counted<'_>) -> Self {
        Self {
            known_files: c.is_whole().then(|| c.ins.all_files()),
            ..Self::default()
        }
    }

    pub fn finish(mut self) -> PeriodStats {
        self.s.authors = self.people.len();
        self.s.files = self.known_files.unwrap_or(self.files.len());
        self.s
    }
}

impl Reducer for Totals {
    fn needs_rows(&self) -> bool {
        self.known_files.is_none()
    }

    fn add(&mut self, c: &Counted<'_>, item: &Item) {
        self.s.commits += 1;
        self.people.insert(item.person);
        for f in &item.rows {
            if !c.ins.skips_id(f.path) {
                self.files.insert(f.path);
            }
        }
        self.s.additions += u64::from(item.lines.add);
        self.s.deletions += u64::from(item.lines.del);
    }
}

/// Commits and lines per axis bucket.
#[derive(Debug)]
pub struct Series {
    points: Vec<(u32, u64, u64)>,
}

impl Series {
    pub fn new(axis: &TimeAxis) -> Self {
        Self {
            points: vec![(0, 0, 0); axis.len()],
        }
    }

    pub fn finish(self, axis: &TimeAxis) -> Vec<SeriesPoint> {
        axis.names()
            .zip(self.points)
            .map(
                |((key, label), (commits, additions, deletions))| SeriesPoint {
                    key,
                    label,
                    commits,
                    additions,
                    deletions,
                },
            )
            .collect()
    }
}

impl Reducer for Series {
    fn add(&mut self, c: &Counted<'_>, item: &Item) {
        if let Some(p) = c
            .axis
            .slot_day(item.day)
            .and_then(|s| self.points.get_mut(s))
        {
            p.0 += 1;
            p.1 += u64::from(item.lines.add);
            p.2 += u64::from(item.lines.del);
        }
    }
}

/// When commits happen, in each commit's own offset.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct When {
    /// `[weekday][hour]`, Monday is 0.
    pub punch_card: [[u32; 24]; 7],
    pub by_weekday: [u32; 7],
    pub by_hour: [u32; 24],
}

impl Reducer for When {
    fn add(&mut self, _: &Counted<'_>, item: &Item) {
        let wd = weekday(item.day);
        let h = item.hour.min(23);
        self.punch_card[wd][h] += 1;
        self.by_weekday[wd] += 1;
        self.by_hour[h] += 1;
    }
}

/// One person's counts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PersonTally {
    pub name: String,
    pub bot: bool,
    pub commits: usize,
    pub additions: u64,
    pub deletions: u64,
    pub active_days: usize,
    /// Commits per axis bucket.
    pub series: Vec<u32>,
}

struct PersonAcc {
    commits: usize,
    add: u64,
    del: u64,
    days: BTreeSet<i64>,
    series: Vec<u32>,
}

/// Per-person counts, most commits first, then name.
pub struct PersonTallies {
    len: usize,
    acc: FastMap<usize, PersonAcc>,
}

impl PersonTallies {
    pub fn new(axis: &TimeAxis) -> Self {
        Self {
            len: axis.len(),
            acc: FastMap::default(),
        }
    }

    pub fn finish(self, c: &Counted<'_>) -> Vec<PersonTally> {
        let mut out: Vec<PersonTally> = self
            .acc
            .into_iter()
            .map(|(p, a)| {
                let person = &c.ins.d.people.list[p];
                PersonTally {
                    name: person.name.clone(),
                    bot: person.bot,
                    commits: a.commits,
                    additions: a.add,
                    deletions: a.del,
                    active_days: a.days.len(),
                    series: a.series,
                }
            })
            .collect();
        out.sort_by(|a, b| b.commits.cmp(&a.commits).then_with(|| a.name.cmp(&b.name)));
        out
    }
}

impl Reducer for PersonTallies {
    fn add(&mut self, c: &Counted<'_>, item: &Item) {
        let len = self.len;
        let a = self.acc.entry(item.person).or_insert_with(|| PersonAcc {
            commits: 0,
            add: 0,
            del: 0,
            days: BTreeSet::new(),
            series: vec![0; len],
        });
        a.commits += 1;
        a.add += u64::from(item.lines.add);
        a.del += u64::from(item.lines.del);
        a.days.insert(item.day);
        if let Some(s) = c.axis.slot_day(item.day).and_then(|s| a.series.get_mut(s)) {
            *s += 1;
        }
    }
}

/// Type or scope counts, overall and per axis bucket.
pub struct LabelTally {
    label: Label,
    total: HashMap<Option<LabelId>, usize>,
    buckets: Vec<HashMap<Option<LabelId>, usize>>,
}

impl LabelTally {
    pub fn new(label: Label, axis: &TimeAxis) -> Self {
        Self {
            label,
            total: HashMap::new(),
            buckets: vec![HashMap::new(); axis.len()],
        }
    }

    pub fn finish(self, c: &Counted<'_>) -> (Vec<LabelCount>, Vec<LabelBucket>) {
        let src = c.ins.source();
        let label = self.label;
        let named = |m: HashMap<Option<LabelId>, usize>| label_counts(src, label, m);
        let buckets = c
            .axis
            .names()
            .zip(self.buckets)
            .map(|((key, name), m)| LabelBucket {
                key,
                label: name,
                counts: named(m),
            })
            .collect();
        (named(self.total), buckets)
    }
}

impl Reducer for LabelTally {
    fn add(&mut self, c: &Counted<'_>, item: &Item) {
        let id = match self.label {
            Label::Kind => item.commit.kind,
            Label::Scope => item.commit.scope,
        };
        *self.total.entry(id).or_default() += 1;
        if let Some(m) = c
            .axis
            .slot_day(item.day)
            .and_then(|s| self.buckets.get_mut(s))
        {
            *m.entry(id).or_default() += 1;
        }
    }
}

/// Label counts by name, most first, then name: the order `label_breakdown` uses.
fn label_counts(
    src: &dyn InsightsSource,
    label: Label,
    m: HashMap<Option<LabelId>, usize>,
) -> Vec<LabelCount> {
    let missing = match label {
        Label::Kind => Selection::OTHER,
        Label::Scope => Selection::NO_SCOPE,
    };
    let mut named: HashMap<String, usize> = HashMap::new();
    for (id, n) in m {
        let name = id
            .and_then(|i| src.label(i))
            .map_or_else(|| missing.to_owned(), |s| s.to_string());
        *named.entry(name).or_default() += n;
    }
    let mut out: Vec<LabelCount> = named
        .into_iter()
        .map(|(label, commits)| LabelCount { label, commits })
        .collect();
    out.sort_by(|a, b| {
        b.commits
            .cmp(&a.commits)
            .then_with(|| a.label.cmp(&b.label))
    });
    out
}

/// Commits by size (added plus removed lines over counted files).
#[derive(Debug, Default)]
pub struct Sizes {
    counts: [usize; 7],
}

impl Sizes {
    pub fn finish(self) -> Vec<SizeBucket> {
        SIZE_LABELS
            .iter()
            .zip(self.counts)
            .map(|(l, commits)| SizeBucket {
                label: (*l).to_owned(),
                commits,
            })
            .collect()
    }
}

impl Reducer for Sizes {
    fn add(&mut self, _: &Counted<'_>, item: &Item) {
        let n = item.lines.add + item.lines.del;
        let slot = SIZE_EDGES
            .iter()
            .position(|e| n <= *e)
            .unwrap_or(SIZE_EDGES.len());
        self.counts[slot] += 1;
    }
}
